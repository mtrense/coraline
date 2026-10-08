#![deny(unsafe_code)]
#![allow(
    clippy::cast_possible_wrap,
    clippy::cast_possible_truncation,
    clippy::collapsible_if,
    clippy::equatable_if_let,
    clippy::indexing_slicing,
    clippy::manual_let_else,
    clippy::match_same_arms,
    clippy::missing_const_for_fn,
    clippy::needless_pass_by_value,
    clippy::option_if_let_else,
    clippy::redundant_clone,
    clippy::redundant_closure_for_method_calls,
    clippy::too_many_arguments,
    clippy::too_many_lines,
    clippy::uninlined_format_args,
    clippy::used_underscore_binding
)]

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use rayon::prelude::*;
use tree_sitter::{Node as TsNode, Parser};

use crate::config::is_language_supported;
use crate::db;
use crate::resolution::{ReferenceResolver, receiver};
use crate::types::{
    CodeGraphConfig, Edge, EdgeKind, ExtractionError, ExtractionErrorSeverity, FileRecord,
    Language, Node, NodeKind, UnresolvedReference, Visibility,
};
use crate::utils::{hash_sha256, node_id_for_symbol};
use tracing::{debug, info, warn};

/// Page size for reading unresolved refs; the resolver visits all pages.
const RESOLVE_BATCH_SIZE: usize = 10_000;

#[cfg(test)]
mod grammar_guard_tests;

#[derive(Debug, Clone, Copy)]
pub enum IndexPhase {
    Scanning,
    Parsing,
    Storing,
    Resolving,
}

#[derive(Debug, Clone)]
pub struct IndexProgress {
    pub phase: IndexPhase,
    pub current: usize,
    pub total: usize,
    pub current_file: Option<String>,
}

#[derive(Debug, Clone)]
pub struct IndexResult {
    pub success: bool,
    pub files_indexed: usize,
    pub files_skipped: usize,
    pub nodes_created: usize,
    pub edges_created: usize,
    pub errors: Vec<ExtractionError>,
    pub duration_ms: u128,
}

#[derive(Debug, Clone)]
pub struct SyncResult {
    pub files_checked: usize,
    pub files_added: usize,
    pub files_modified: usize,
    pub files_removed: usize,
    pub nodes_updated: usize,
    pub duration_ms: u128,
}

struct ParsedFile {
    file_record: FileRecord,
    nodes: Vec<Node>,
    edges: Vec<Edge>,
    unresolved_refs: Vec<UnresolvedReference>,
    node_count: usize,
    edge_count: usize,
}

fn parse_file_only(
    project_root: &Path,
    config: &CodeGraphConfig,
    existing_hashes: &std::collections::HashMap<String, String>,
    relative_path: &str,
) -> Option<ParsedFile> {
    let full_path = project_root.join(relative_path);
    let content = fs::read_to_string(&full_path).ok()?;

    if (content.len() as u64) > config.max_file_size {
        return None;
    }

    let language = detect_language(relative_path);
    if !is_language_supported(&language) {
        return None;
    }

    let content_hash = hash_sha256(&content);
    if existing_hashes
        .get(relative_path)
        .is_some_and(|h| *h == content_hash)
    {
        return None; // unchanged
    }

    let file_name = Path::new(relative_path)
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or(relative_path);
    let qualified_name = relative_path.to_string();
    let node_id = node_id_for_symbol(relative_path, "file", &qualified_name, 1, 0);
    let file_node_id = node_id.clone();

    let now_ms = now_millis();
    let mut nodes = Vec::new();
    let file_node = Node {
        id: node_id,
        kind: NodeKind::File,
        name: file_name.to_string(),
        qualified_name,
        file_path: relative_path.to_string(),
        language,
        start_line: 1,
        end_line: 1,
        start_column: 0,
        end_column: 0,
        docstring: None,
        signature: None,
        visibility: None,
        is_exported: false,
        is_async: false,
        is_static: false,
        is_abstract: false,
        decorators: None,
        type_parameters: None,
        cluster_id: None,
        updated_at: now_ms,
    };
    nodes.push(file_node);

    let (mut extracted_nodes, edges, unresolved_refs) = extract_nodes(
        project_root,
        relative_path,
        &content,
        language,
        now_ms,
        &file_node_id,
    );
    nodes.append(&mut extracted_nodes);

    let metadata = fs::metadata(&full_path).ok()?;
    let file_record = FileRecord {
        path: relative_path.to_string(),
        content_hash,
        language,
        size: metadata.len(),
        modified_at: metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX)),
        indexed_at: now_ms,
        node_count: nodes.len() as i64,
        errors: None,
    };

    let node_count = nodes.len();
    let edge_count = edges.len();
    Some(ParsedFile {
        file_record,
        nodes,
        edges,
        unresolved_refs,
        node_count,
        edge_count,
    })
}

pub fn index_all(
    project_root: &Path,
    config: &CodeGraphConfig,
    force: bool,
    on_progress: Option<&dyn Fn(IndexProgress)>,
) -> std::io::Result<IndexResult> {
    let span = tracing::info_span!("index_all", ?force, root = %project_root.display());
    let _enter = span.enter();
    let start = Instant::now();
    let mut errors = Vec::new();
    let mut files_indexed = 0;
    let mut nodes_created = 0;
    let mut edges_created = 0;

    let files = scan_directory(project_root, config, |current, file| {
        if let Some(cb) = on_progress {
            cb(IndexProgress {
                phase: IndexPhase::Scanning,
                current,
                total: 0,
                current_file: Some(file.to_string()),
            });
        }
    });

    let mut conn = db::open_database(project_root)?;
    if force {
        db::clear_database(&conn)?;
    }

    // Pre-fetch existing file hashes to avoid DB access in the parallel parse phase.
    let existing_hashes: std::collections::HashMap<String, String> = if force {
        std::collections::HashMap::new()
    } else {
        db::list_files(&conn)?
            .into_iter()
            .map(|f| (f.path, f.content_hash))
            .collect()
    };

    if let Some(cb) = on_progress {
        cb(IndexProgress {
            phase: IndexPhase::Parsing,
            current: 0,
            total: files.len(),
            current_file: None,
        });
    }

    info!(total_files = files.len(), "starting parallel parse phase");

    // Phase 1: Parse all files in parallel (CPU-bound, no DB access).
    let parsed: Vec<ParsedFile> = files
        .par_iter()
        .filter_map(|file| parse_file_only(project_root, config, &existing_hashes, file))
        .collect();

    let files_skipped = files.len().saturating_sub(parsed.len());
    info!(
        parsed = parsed.len(),
        skipped = files_skipped,
        "parse phase complete"
    );

    if let Some(cb) = on_progress {
        cb(IndexProgress {
            phase: IndexPhase::Storing,
            current: 0,
            total: parsed.len(),
            current_file: None,
        });
    }

    // Phase 2: Store results sequentially (SQLite does not support concurrent writes).
    for (idx, parsed_file) in parsed.into_iter().enumerate() {
        // Delete the old record before inserting the new batch so foreign keys are clean.
        let _ = db::delete_file(&mut conn, &parsed_file.file_record.path);

        if let Some(cb) = on_progress {
            cb(IndexProgress {
                phase: IndexPhase::Storing,
                current: idx + 1,
                total: files.len(),
                current_file: Some(parsed_file.file_record.path.clone()),
            });
        }

        let path = parsed_file.file_record.path.clone();
        debug!(file = %path, nodes = parsed_file.node_count, edges = parsed_file.edge_count, "storing file");
        match db::store_file_batch(
            &mut conn,
            &parsed_file.file_record,
            &parsed_file.nodes,
            &parsed_file.edges,
            &parsed_file.unresolved_refs,
        ) {
            Ok(()) => {
                files_indexed += 1;
                nodes_created += parsed_file.node_count;
                edges_created += parsed_file.edge_count;
            }
            Err(err) => {
                warn!(file = %path, error = %err, "failed to store file");
                errors.push(ExtractionError {
                    message: err.to_string(),
                    line: None,
                    column: None,
                    severity: ExtractionErrorSeverity::Error,
                    code: None,
                });
            }
        }
    }

    if let Err(err) =
        ReferenceResolver::resolve_unresolved(&mut conn, project_root, RESOLVE_BATCH_SIZE)
    {
        warn!(error = %err, "reference resolver failed");
        errors.push(ExtractionError {
            message: format!("Resolver failed: {err}"),
            line: None,
            column: None,
            severity: ExtractionErrorSeverity::Warning,
            code: Some("resolver_failed".to_string()),
        });
    }

    info!(
        files_indexed,
        files_skipped,
        nodes_created,
        edges_created,
        duration_ms = start.elapsed().as_millis(),
        "index_all complete"
    );

    Ok(IndexResult {
        success: errors
            .iter()
            .all(|e| e.severity != ExtractionErrorSeverity::Error),
        files_indexed,
        files_skipped,
        nodes_created,
        edges_created,
        errors,
        duration_ms: start.elapsed().as_millis(),
    })
}

pub fn sync(
    project_root: &Path,
    config: &CodeGraphConfig,
    on_progress: Option<&dyn Fn(IndexProgress)>,
) -> std::io::Result<SyncResult> {
    let span = tracing::info_span!("sync", root = %project_root.display());
    let _enter = span.enter();
    let start = Instant::now();
    let mut conn = db::open_database(project_root)?;

    let current_files: HashSet<String> = scan_directory(project_root, config, |_current, _file| {})
        .into_iter()
        .collect();
    let tracked_files = db::list_files(&conn)?;

    let mut files_added = 0;
    let mut files_modified = 0;
    let mut files_removed = 0;
    let mut nodes_updated = 0;

    for tracked in &tracked_files {
        if !current_files.contains(&tracked.path) {
            db::delete_file(&mut conn, &tracked.path)?;
            files_removed += 1;
        }
    }

    for (idx, file) in current_files.iter().enumerate() {
        if let Some(cb) = on_progress {
            cb(IndexProgress {
                phase: IndexPhase::Parsing,
                current: idx + 1,
                total: current_files.len(),
                current_file: Some(file.clone()),
            });
        }

        let full_path = project_root.join(file);
        let content = fs::read_to_string(&full_path)?;
        let content_hash = hash_sha256(&content);
        let tracked = tracked_files.iter().find(|f| f.path == *file);

        if let Some(tracked) = tracked {
            if tracked.content_hash != content_hash {
                match index_file(project_root, config, &mut conn, file) {
                    Ok(Some((node_count, _))) => {
                        files_modified += 1;
                        nodes_updated += node_count;
                    }
                    Ok(None) => {}
                    Err(err) => {
                        warn!(file = %file, error = %err, "failed to sync file");
                    }
                }
            }
        } else {
            match index_file(project_root, config, &mut conn, file) {
                Ok(Some((node_count, _))) => {
                    files_added += 1;
                    nodes_updated += node_count;
                }
                Ok(None) => {}
                Err(err) => {
                    warn!(file = %file, error = %err, "failed to sync file");
                }
            }
        }
    }

    let _ = ReferenceResolver::resolve_unresolved(&mut conn, project_root, RESOLVE_BATCH_SIZE);

    info!(
        files_added,
        files_modified,
        files_removed,
        nodes_updated,
        duration_ms = start.elapsed().as_millis(),
        "sync complete"
    );

    Ok(SyncResult {
        files_checked: current_files.len(),
        files_added,
        files_modified,
        files_removed,
        nodes_updated,
        duration_ms: start.elapsed().as_millis(),
    })
}

fn index_file(
    project_root: &Path,
    config: &CodeGraphConfig,
    conn: &mut rusqlite::Connection,
    relative_path: &str,
) -> std::io::Result<Option<(usize, usize)>> {
    let full_path = project_root.join(relative_path);
    let content = fs::read_to_string(&full_path)?;

    if (content.len() as u64) > config.max_file_size {
        return Ok(None);
    }

    let language = detect_language(relative_path);
    if !is_language_supported(&language) {
        return Ok(None);
    }

    let content_hash = hash_sha256(&content);
    if let Some(existing) = db::get_file_record(conn, relative_path)? {
        if existing.content_hash == content_hash {
            return Ok(None);
        }
        db::delete_file(conn, relative_path)?;
    }

    let file_name = Path::new(relative_path)
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or(relative_path);
    let qualified_name = relative_path.to_string();
    let node_id = node_id_for_symbol(relative_path, "file", &qualified_name, 1, 0);
    let file_node_id = node_id.clone();

    let now_ms = now_millis();
    let mut nodes = Vec::new();
    let file_node = Node {
        id: node_id,
        kind: NodeKind::File,
        name: file_name.to_string(),
        qualified_name,
        file_path: relative_path.to_string(),
        language,
        start_line: 1,
        end_line: 1,
        start_column: 0,
        end_column: 0,
        docstring: None,
        signature: None,
        visibility: None,
        is_exported: false,
        is_async: false,
        is_static: false,
        is_abstract: false,
        decorators: None,
        type_parameters: None,
        cluster_id: None,
        updated_at: now_ms,
    };
    nodes.push(file_node);

    let (mut extracted_nodes, extracted_edges, unresolved_refs) = extract_nodes(
        project_root,
        relative_path,
        &content,
        language,
        now_ms,
        &file_node_id,
    );
    nodes.append(&mut extracted_nodes);

    if !nodes.is_empty() {
        db::insert_nodes(conn, &nodes)?;
    }
    if !extracted_edges.is_empty() {
        db::insert_edges(conn, &extracted_edges)?;
    }
    if !unresolved_refs.is_empty() {
        db::insert_unresolved_refs(conn, &unresolved_refs)?;
    }

    let metadata = fs::metadata(&full_path)?;
    let file_record = FileRecord {
        path: relative_path.to_string(),
        content_hash,
        language,
        size: metadata.len(),
        modified_at: metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX)),
        indexed_at: now_ms,
        node_count: nodes.len() as i64,
        errors: None,
    };
    db::upsert_file(conn, &file_record)?;

    Ok(Some((nodes.len(), extracted_edges.len())))
}

fn extract_nodes(
    project_root: &Path,
    file_path: &str,
    source: &str,
    language: Language,
    now_ms: i64,
    root_id: &str,
) -> (Vec<Node>, Vec<Edge>, Vec<UnresolvedReference>) {
    let mut parser = Parser::new();
    let ts_lang = match language_to_parser(language) {
        Some(ts_lang) => ts_lang,
        None => return (Vec::new(), Vec::new(), Vec::new()),
    };

    if parser.set_language(&ts_lang).is_err() {
        return (Vec::new(), Vec::new(), Vec::new());
    }

    let tree = match parser.parse(source, None) {
        Some(tree) => tree,
        None => return (Vec::new(), Vec::new(), Vec::new()),
    };

    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    let mut symbol_index = SymbolIndex::default();
    let mut stack = Vec::new();
    let mut unresolved_refs = Vec::new();
    walk_tree_collect(
        tree.root_node(),
        source,
        project_root,
        file_path,
        language,
        &mut stack,
        Some(root_id.to_string()),
        &mut nodes,
        &mut edges,
        &mut symbol_index,
        now_ms,
    );
    if language == Language::Kotlin {
        // Kotlin has no export syntax: every top-level declaration that is
        // not `private` is visible to other files.
        let root = tree.root_node();
        for decl in root.children(&mut root.walk()) {
            add_export_nodes(
                &decl,
                source,
                language,
                file_path,
                root_id.to_string(),
                &mut nodes,
                &mut edges,
                now_ms,
            );
        }
    }
    walk_tree_calls(
        tree.root_node(),
        source,
        file_path,
        language,
        &symbol_index,
        &mut edges,
        &mut unresolved_refs,
        &mut Vec::new(),
    );
    (nodes, edges, unresolved_refs)
}

fn language_to_parser(language: Language) -> Option<tree_sitter::Language> {
    match language {
        Language::Rust => Some(tree_sitter::Language::new(tree_sitter_rust::LANGUAGE)),
        Language::JavaScript | Language::Jsx => {
            Some(tree_sitter::Language::new(tree_sitter_javascript::LANGUAGE))
        }
        Language::TypeScript => Some(tree_sitter::Language::new(
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT,
        )),
        Language::Tsx => Some(tree_sitter::Language::new(
            tree_sitter_typescript::LANGUAGE_TSX,
        )),
        Language::Python => Some(tree_sitter::Language::new(tree_sitter_python::LANGUAGE)),
        Language::Go => Some(tree_sitter::Language::new(tree_sitter_go::LANGUAGE)),
        Language::Java => Some(tree_sitter::Language::new(tree_sitter_java::LANGUAGE)),
        Language::C => Some(tree_sitter::Language::new(tree_sitter_c::LANGUAGE)),
        Language::Cpp => Some(tree_sitter::Language::new(tree_sitter_cpp::LANGUAGE)),
        Language::CSharp => Some(tree_sitter::Language::new(tree_sitter_c_sharp::LANGUAGE)),
        Language::Ruby => Some(tree_sitter::Language::new(tree_sitter_ruby::LANGUAGE)),
        Language::Php => Some(tree_sitter::Language::new(tree_sitter_php::LANGUAGE_PHP)),
        Language::Swift => Some(tree_sitter::Language::new(tree_sitter_swift::LANGUAGE)),
        Language::Kotlin => Some(tree_sitter::Language::new(tree_sitter_kotlin_ng::LANGUAGE)),
        Language::Markdown => Some(tree_sitter_markdown_updated::language()),
        // Unsupported languages (see `config::is_language_supported`) and
        // languages without a tree-sitter grammar. Blazor (`.razor`) is
        // indexed at file level only: it is Razor markup, not C#, so the C#
        // grammar yields only ERROR nodes.
        _ => None,
    }
}

#[derive(Debug, Default)]
struct SymbolIndex {
    by_name: HashMap<String, Vec<String>>,
    by_key: HashMap<String, String>,
    callable_ids: HashSet<String>,
    /// Enclosing type / namespace names per callable id, outermost first.
    containers: HashMap<String, Vec<String>>,
}

/// Read visibility from a declaration tree-sitter node.
///
/// Returns `(visibility, is_exported)`. `is_exported` is true iff the
/// node is exposed to consumers outside the declaring module — i.e.,
/// reachable from a downstream crate's `use` statement or a library
/// consumer's import. Restricted-visibility keywords (`pub(crate)`,
/// `pub(super)`, `pub(in path)`) are exported = false because they
/// are NOT part of the public API surface, even though they ARE
/// reachable from inside the crate.
///
/// - **Rust** — looks for a `visibility_modifier` child. Plain `pub`
///   maps to `(Visibility::Public, true)`; qualified forms
///   (`pub(crate)`, `pub(super)`, `pub(in path)`) map to
///   `(Visibility::Internal, false)`; missing modifier is `None`.
/// - **TypeScript / JavaScript** — walks ancestors for
///   `export_statement`. Maps to `(Visibility::Public, true)`.
/// - **Java / C#** — walks children for a `modifiers` node
///   containing `public`. Maps to `(Visibility::Public, true)`.
///
/// Languages without a keyword-based visibility concept (Python,
/// Ruby, etc.) return `None`. Convention-based visibility (e.g.
/// Python's leading-underscore) is left to a follow-up.
fn read_declaration_visibility(
    node: TsNode,
    language: Language,
    source: &str,
) -> Option<(Visibility, bool)> {
    match language {
        Language::Rust => {
            // `visibility_modifier` is a child, not a field.
            let vis = child_of_kind(&node, "visibility_modifier")?;
            let text = vis.utf8_text(source.as_bytes()).ok()?.trim();
            if text == "pub" {
                Some((Visibility::Public, true))
            } else if text.starts_with("pub(") {
                Some((Visibility::Internal, false))
            } else {
                None
            }
        }
        Language::TypeScript | Language::Tsx | Language::JavaScript => {
            let mut current = node;
            loop {
                if current.kind() == "export_statement" {
                    return Some((Visibility::Public, true));
                }
                let parent = current.parent()?;
                current = parent;
            }
        }
        // Java wraps modifiers in a `modifiers` node; C# has one `modifier`
        // child per keyword.
        Language::Java | Language::CSharp => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if matches!(child.kind(), "modifiers" | "modifier") {
                    let text = child.utf8_text(source.as_bytes()).ok()?;
                    if text.contains("public") {
                        return Some((Visibility::Public, true));
                    }
                }
            }
            None
        }
        _ => None,
    }
}

fn walk_tree_collect(
    node: TsNode,
    source: &str,
    project_root: &Path,
    file_path: &str,
    language: Language,
    stack: &mut Vec<String>,
    parent_id: Option<String>,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
    symbol_index: &mut SymbolIndex,
    now_ms: i64,
) {
    let (kind, is_container) = node_kind(&node, source, language);

    if let Some(NodeKind::Import) = kind {
        if let Some(parent_id) = parent_id.clone() {
            add_import_nodes(
                &node,
                source,
                language,
                file_path,
                parent_id.clone(),
                nodes,
                edges,
                now_ms,
            );
            // `pub use a::B;` also re-exports `B`.
            if language == Language::Rust
                && read_declaration_visibility(node, language, source)
                    .is_some_and(|(_, exported)| exported)
            {
                add_export_nodes(
                    &node, source, language, file_path, parent_id, nodes, edges, now_ms,
                );
            }
            return;
        }
    }

    if let Some(NodeKind::Module) = kind {
        if let Some(parent_id) = parent_id.clone() {
            add_module_node(
                &node,
                source,
                project_root,
                language,
                file_path,
                parent_id,
                nodes,
                edges,
                now_ms,
            );
            return;
        }
    }

    let mut handled_export = false;
    if let Some(NodeKind::Export) = kind {
        if let Some(parent_id) = parent_id.clone() {
            add_export_nodes(
                &node, source, language, file_path, parent_id, nodes, edges, now_ms,
            );
            handled_export = true;
        }
    }

    let name = if handled_export {
        None
    } else {
        match kind {
            Some(_) => node_name(&node, source, language),
            None => None,
        }
    };

    let mut next_parent_id = parent_id.clone();

    if let (Some(kind), Some(name)) = (kind, name.clone()) {
        let qualified_name = if stack.is_empty() {
            format!("{}::{}", file_path, name)
        } else {
            format!("{}::{}::{}", file_path, stack.join("::"), name)
        };
        let id = node_id_for_symbol(
            file_path,
            &format!("{:?}", kind).to_ascii_lowercase(),
            &qualified_name,
            node.start_position().row as i64 + 1,
            node.start_position().column as i64,
        );
        let start = node.start_position();
        let end = node.end_position();
        let (visibility, is_exported) = match read_declaration_visibility(node, language, source) {
            Some((text, exported)) => (Some(text), exported),
            None => (None, false),
        };

        nodes.push(Node {
            id: id.clone(),
            kind,
            name: name.clone(),
            qualified_name,
            file_path: file_path.to_string(),
            language,
            start_line: start.row as i64 + 1,
            end_line: end.row as i64 + 1,
            start_column: start.column as i64,
            end_column: end.column as i64,
            docstring: None,
            signature: None,
            visibility,
            is_exported,
            is_async: false,
            is_static: false,
            is_abstract: false,
            decorators: None,
            type_parameters: None,
            updated_at: now_ms,
            cluster_id: None,
        });

        if is_callable_kind(kind) {
            let key = node_key(kind, start, &name);
            symbol_index.by_key.insert(key, id.clone());
            symbol_index
                .by_name
                .entry(name.clone())
                .or_default()
                .push(id.clone());
            symbol_index.callable_ids.insert(id.clone());
            symbol_index.containers.insert(id.clone(), stack.clone());
        }

        if let Some(parent_id) = parent_id.clone() {
            edges.push(Edge {
                source: parent_id.clone(),
                target: id.clone(),
                kind: EdgeKind::Contains,
                metadata: None,
                line: Some(start.row as i64 + 1),
                column: Some(start.column as i64),
                confidence: 1.0,
                process_id: None,
            });

            if kind == NodeKind::Import {
                edges.push(Edge {
                    source: parent_id.clone(),
                    target: id.clone(),
                    kind: EdgeKind::Imports,
                    metadata: None,
                    line: Some(start.row as i64 + 1),
                    column: Some(start.column as i64),
                    confidence: 1.0,
                    process_id: None,
                });
            }

            if kind == NodeKind::Export {
                edges.push(Edge {
                    source: parent_id.clone(),
                    target: id.clone(),
                    kind: EdgeKind::Exports,
                    metadata: None,
                    line: Some(start.row as i64 + 1),
                    column: Some(start.column as i64),
                    confidence: 1.0,
                    process_id: None,
                });
            }
        }

        if is_container {
            stack.push(name);
            next_parent_id = Some(id);
        }
    }

    for child in node.children(&mut node.walk()) {
        walk_tree_collect(
            child,
            source,
            project_root,
            file_path,
            language,
            stack,
            next_parent_id.clone(),
            nodes,
            edges,
            symbol_index,
            now_ms,
        );
    }

    if is_container && name.is_some() {
        stack.pop();
    }
}

fn walk_tree_calls(
    node: TsNode,
    source: &str,
    _file_path: &str,
    language: Language,
    symbol_index: &SymbolIndex,
    edges: &mut Vec<Edge>,
    unresolved_refs: &mut Vec<UnresolvedReference>,
    scope_stack: &mut Vec<String>,
) {
    let (kind, _) = node_kind(&node, source, language);
    let name = if kind.is_some() {
        node_name(&node, source, language)
    } else {
        None
    };

    if let (Some(kind), Some(name)) = (kind, name.clone()) {
        if is_callable_kind(kind) {
            let key = node_key(kind, node.start_position(), &name);
            if let Some(id) = symbol_index.by_key.get(&key) {
                scope_stack.push(id.clone());
            }
        }
    }

    // Ruby `require` calls are imports.
    if kind != Some(NodeKind::Import) && is_call_expression(node.kind(), language) {
        if let Some(source_id) = scope_stack.last() {
            if let Some(callee_name) = call_name(&node, source, language) {
                let start = node.start_position();
                let qualifier = call_qualifier(&node, source, language);
                match same_file_targets(symbol_index, &callee_name, qualifier.as_deref(), language)
                {
                    Some(targets) if targets.len() == 1 => {
                        edges.push(Edge {
                            source: source_id.clone(),
                            target: targets[0].clone(),
                            kind: EdgeKind::Calls,
                            metadata: None,
                            line: Some(start.row as i64 + 1),
                            column: Some(start.column as i64),
                            confidence: 1.0,
                            process_id: None,
                        });
                    }
                    Some(targets) => {
                        unresolved_refs.push(UnresolvedReference {
                            from_node_id: source_id.clone(),
                            reference_name: callee_name.clone(),
                            reference_kind: EdgeKind::Calls,
                            line: start.row as i64 + 1,
                            column: start.column as i64,
                            candidates: Some(targets.clone()),
                            qualifier,
                        });
                    }
                    None => {
                        unresolved_refs.push(UnresolvedReference {
                            from_node_id: source_id.clone(),
                            reference_name: callee_name.clone(),
                            reference_kind: EdgeKind::Calls,
                            line: start.row as i64 + 1,
                            column: start.column as i64,
                            candidates: None,
                            qualifier,
                        });
                    }
                }
            }
        }
    }

    for child in node.children(&mut node.walk()) {
        walk_tree_calls(
            child,
            source,
            _file_path,
            language,
            symbol_index,
            edges,
            unresolved_refs,
            scope_stack,
        );
    }

    if let (Some(kind), Some(name)) = (kind, name) {
        if is_callable_kind(kind) {
            let key = node_key(kind, node.start_position(), &name);
            if symbol_index.by_key.contains_key(&key) {
                scope_stack.pop();
            }
        }
    }
}

/// Same-file declarations a call `qualifier.name()` resolves to, when the
/// file alone can decide: unqualified calls, `this` / `self` receivers and
/// qualifiers naming a type declared here (`Circle.create()`). Other
/// qualifiers (`String.format`, `util.f`) may name an import or a type from
/// elsewhere → `None`, left to the resolver.
fn same_file_targets(
    symbol_index: &SymbolIndex,
    name: &str,
    qualifier: Option<&str>,
    language: Language,
) -> Option<Vec<String>> {
    let targets = symbol_index.by_name.get(name)?;
    let Some(qualifier) = qualifier else {
        return Some(targets.clone());
    };
    let containers = |id: &String| {
        symbol_index
            .containers
            .get(id)
            .map_or(&[][..], Vec::as_slice)
    };
    let decided_here = receiver::is_self_like(receiver::last_segment(qualifier))
        || targets
            .iter()
            .any(|id| receiver::names_container(qualifier, containers(id)));
    let admitted: Vec<String> = targets
        .iter()
        .filter(|id| receiver::qualifier_admits(language, qualifier, containers(id)))
        .cloned()
        .collect();
    (decided_here && !admitted.is_empty()).then_some(admitted)
}

fn node_name(node: &TsNode, source: &str, language: Language) -> Option<String> {
    if language == Language::Kotlin {
        if let Some(name) = kotlin_node_name(node, source) {
            return Some(name);
        }
    }

    if matches!(language, Language::C | Language::Cpp) && node.kind() == "function_definition" {
        return c_function_name(node)
            .and_then(|n| n.utf8_text(source.as_bytes()).ok())
            .map(|s| s.to_string());
    }

    if matches!(
        language,
        Language::JavaScript | Language::Jsx | Language::TypeScript | Language::Tsx
    ) && matches!(node.kind(), "arrow_function" | "function_expression")
    {
        return js_function_value_name(node, source);
    }

    // Swift `deinit` has no name field.
    if language == Language::Swift && node.kind() == "deinit_declaration" {
        return Some("deinit".to_string());
    }

    let name_node = node
        .child_by_field_name("name")
        .or_else(|| node.child_by_field_name("identifier"))
        .or_else(|| node.child_by_field_name("property"))
        .or_else(|| node.child_by_field_name("tag_name"));

    name_node
        .and_then(|n| n.utf8_text(source.as_bytes()).ok())
        .map(|s| s.to_string())
}

/// Name of a JS/TS function value: its own name (`function g() {}`), else
/// the variable it initialises (`const f = () => …`). Other anonymous
/// functions (callbacks, IIFEs) have no name, so calls inside them stay
/// attributed to the enclosing function.
fn js_function_value_name(node: &TsNode, source: &str) -> Option<String> {
    let name_node = node.child_by_field_name("name").or_else(|| {
        node.parent()
            .filter(|p| p.kind() == "variable_declarator")
            .filter(|p| p.child_by_field_name("value") == Some(*node))
            .and_then(|p| p.child_by_field_name("name"))
            .filter(|n| n.kind() == "identifier")
    })?;
    name_node
        .utf8_text(source.as_bytes())
        .ok()
        .map(|s| s.to_string())
}

/// Name node of a C/C++ `function_definition`. The name is not a field of the
/// definition but sits at the end of its `declarator` chain, e.g.
/// `pointer_declarator > function_declarator > identifier` for `int *f()`,
/// or `qualified_identifier` (`Shape::area`) in C++.
fn c_function_name<'tree>(node: &TsNode<'tree>) -> Option<TsNode<'tree>> {
    let mut current = node.child_by_field_name("declarator")?;
    loop {
        current = match current.kind() {
            "identifier" | "field_identifier" | "destructor_name" | "operator_name" => {
                return Some(current);
            }
            "qualified_identifier" | "template_function" => current.child_by_field_name("name")?,
            // Wrappers without a `declarator` field.
            "parenthesized_declarator" | "reference_declarator" | "attributed_declarator" => {
                current
                    .named_children(&mut current.walk())
                    .find(|c| !matches!(c.kind(), "ms_call_modifier" | "attribute_declaration"))?
            }
            _ => current.child_by_field_name("declarator")?,
        };
    }
}

/// Whether a C++ `function_definition` defines a method: either inline in a
/// class body or out of line with a qualified name (`void Shape::area()`).
fn cpp_is_method(node: &TsNode) -> bool {
    if node
        .parent()
        .is_some_and(|p| p.kind() == "field_declaration_list")
    {
        return true;
    }
    let mut current = node.child_by_field_name("declarator");
    while let Some(declarator) = current {
        if declarator.kind() == "qualified_identifier" {
            return true;
        }
        current = declarator.child_by_field_name("declarator");
    }
    false
}

/// Callee of a C/C++ `call_expression`: the `function` expression, unwrapped
/// to its member / unqualified name (`obj.run`, `p->go`, `std::sort`,
/// `calc<int>`). Calls through function-pointer expressions have no name.
fn c_callee<'tree>(node: &TsNode<'tree>) -> Option<TsNode<'tree>> {
    let mut current = node.child_by_field_name("function")?;
    loop {
        current = match current.kind() {
            "identifier" | "field_identifier" | "destructor_name" | "operator_name" => {
                return Some(current);
            }
            "field_expression" => current.child_by_field_name("field")?,
            "qualified_identifier" | "template_function" | "template_method" => {
                current.child_by_field_name("name")?
            }
            _ => return None,
        };
    }
}

fn child_of_kind<'tree>(node: &TsNode<'tree>, kind: &str) -> Option<TsNode<'tree>> {
    node.children(&mut node.walk()).find(|c| c.kind() == kind)
}

/// Package of a Kotlin `package_header` node.
fn kotlin_package_name(node: &TsNode, source: &str) -> Option<String> {
    child_of_kind(node, "qualified_identifier")
        .and_then(|n| n.utf8_text(source.as_bytes()).ok())
        .map(|s| s.to_string())
}

/// Names of Kotlin declarations that have no `name` field.
fn kotlin_node_name(node: &TsNode, source: &str) -> Option<String> {
    let name_node = match node.kind() {
        // `val x = …`; destructuring declarations (`val (a, b) = …`) have
        // no single name and are skipped.
        "property_declaration" => child_of_kind(node, "variable_declaration")
            .and_then(|decl| child_of_kind(&decl, "identifier"))?,
        "enum_entry" => child_of_kind(node, "identifier")?,
        "type_alias" => node.child_by_field_name("type")?,
        "secondary_constructor" => return Some("constructor".to_string()),
        "companion_object" => {
            return Some(
                node.child_by_field_name("name")
                    .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                    .unwrap_or("Companion")
                    .to_string(),
            );
        }
        _ => return None,
    };
    name_node
        .utf8_text(source.as_bytes())
        .ok()
        .map(|s| s.to_string())
}

#[derive(Debug, Clone)]
struct ImportSymbol {
    local_name: String,
    module_path: String,
    export_name: Option<String>,
}

#[derive(Debug, Clone)]
struct ExportSymbol {
    name: String,
    module_path: Option<String>,
}

fn add_import_nodes(
    node: &TsNode,
    source: &str,
    language: Language,
    file_path: &str,
    parent_id: String,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
    now_ms: i64,
) {
    let imports = import_symbols(node, source, language);
    if imports.is_empty() {
        return;
    }

    let start = node.start_position();
    let end = node.end_position();

    for import in imports {
        let qualified_name = format!(
            "{}::import::{}::{}",
            file_path, import.local_name, import.module_path
        );
        let id = node_id_for_symbol(
            file_path,
            "import",
            &qualified_name,
            start.row as i64 + 1,
            start.column as i64,
        );
        let signature = build_import_signature(&import.module_path, import.export_name.as_deref());

        nodes.push(Node {
            id: id.clone(),
            kind: NodeKind::Import,
            name: import.local_name,
            qualified_name,
            file_path: file_path.to_string(),
            language,
            start_line: start.row as i64 + 1,
            end_line: end.row as i64 + 1,
            start_column: start.column as i64,
            end_column: end.column as i64,
            docstring: None,
            signature: Some(signature),
            visibility: None,
            is_exported: false,
            is_async: false,
            is_static: false,
            is_abstract: false,
            decorators: None,
            type_parameters: None,
            updated_at: now_ms,
            cluster_id: None,
        });

        edges.push(Edge {
            source: parent_id.clone(),
            target: id.clone(),
            kind: EdgeKind::Contains,
            metadata: None,
            line: Some(start.row as i64 + 1),
            column: Some(start.column as i64),
            confidence: 1.0,
            process_id: None,
        });
        edges.push(Edge {
            source: parent_id.clone(),
            target: id,
            kind: EdgeKind::Imports,
            metadata: None,
            line: Some(start.row as i64 + 1),
            column: Some(start.column as i64),
            confidence: 1.0,
            process_id: None,
        });
    }
}

fn import_symbols(node: &TsNode, source: &str, language: Language) -> Vec<ImportSymbol> {
    match language {
        Language::Php => return php_import_symbols(node, source),
        Language::Go => return go_import_symbols(node, source),
        Language::Rust => return rust_use_symbols(node, source),
        Language::CSharp => return csharp_using_symbols(node, source).into_iter().collect(),
        Language::Python => return python_import_symbols(node, source),
        Language::Ruby => return ruby_require_symbols(node, source),
        _ => {}
    }
    let Some(module_path) = import_module_path(node, source, language) else {
        return Vec::new();
    };

    match language {
        // === JavaScript/TypeScript family ===
        Language::JavaScript | Language::Jsx | Language::TypeScript | Language::Tsx => {
            let mut imports = Vec::new();
            if let Some(clause) = node
                .children(&mut node.walk())
                .find(|c| c.kind() == "import_clause")
            {
                collect_import_symbols(clause, source, &module_path, &mut imports);
            }

            if imports.is_empty() {
                imports.push(ImportSymbol {
                    local_name: module_path.clone(),
                    module_path,
                    export_name: None,
                });
            }
            imports
        }

        // === Java ===
        // `import a.b.*;` / `import static a.B.*;` bind every member.
        Language::Java if child_of_kind(node, "asterisk").is_some() => vec![ImportSymbol {
            local_name: "*".to_string(),
            module_path,
            export_name: None,
        }],
        Language::Java => {
            let last_part = module_path
                .rsplit('.')
                .next()
                .unwrap_or(&module_path)
                .to_string();
            vec![ImportSymbol {
                local_name: last_part.clone(),
                module_path,
                export_name: Some(last_part),
            }]
        }

        // === C/C++ ===
        Language::C | Language::Cpp => {
            let name = module_path
                .trim_end_matches(".h")
                .trim_end_matches(".hpp")
                .split('/')
                .next_back()
                .unwrap_or(&module_path)
                .to_string();
            vec![ImportSymbol {
                local_name: name.clone(),
                module_path,
                export_name: Some(name),
            }]
        }

        // === Swift ===
        Language::Swift => {
            vec![ImportSymbol {
                local_name: module_path.clone(),
                module_path,
                export_name: None,
            }]
        }

        // === Kotlin: `import a.b.C`, `import a.b.C as D`, `import a.b.*` ===
        Language::Kotlin => {
            let mut cursor = node.walk();
            let children: Vec<TsNode> = node.children(&mut cursor).collect();
            if children.iter().any(|c| c.kind() == "*") {
                return vec![ImportSymbol {
                    local_name: "*".to_string(),
                    module_path,
                    export_name: None,
                }];
            }

            let last_part = module_path
                .rsplit('.')
                .next()
                .unwrap_or(&module_path)
                .to_string();
            let alias = children
                .iter()
                .find(|c| c.kind() == "identifier")
                .and_then(|c| c.utf8_text(source.as_bytes()).ok())
                .map(|s| s.to_string());
            vec![ImportSymbol {
                local_name: alias.unwrap_or_else(|| last_part.clone()),
                module_path,
                export_name: Some(last_part),
            }]
        }

        // Blazor, markup and unsupported languages: no imports
        _ => Vec::new(),
    }
}

/// Imports of a Rust `use_declaration`, one per bound name: `use a::B;`,
/// `use a::B as C;`, `use a::{B, c::{D, self}};`, `use a::*;`. The module
/// path is the full path of the item (`a::c::D`); aliased imports record the
/// original name as `export_name`.
fn rust_use_symbols(node: &TsNode, source: &str) -> Vec<ImportSymbol> {
    let mut imports = Vec::new();
    if let Some(argument) = node.child_by_field_name("argument") {
        collect_rust_use(argument, source, "", &mut imports);
    }
    imports
}

fn collect_rust_use(node: TsNode, source: &str, prefix: &str, imports: &mut Vec<ImportSymbol>) {
    let text = |n: TsNode| {
        n.utf8_text(source.as_bytes())
            .ok()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    // `prefix::path`, with `a::{self}` meaning `a`.
    let join = |path: Option<String>| {
        let full = match path {
            Some(path) if prefix.is_empty() => path,
            Some(path) => format!("{prefix}::{path}"),
            None => prefix.to_string(),
        };
        full.strip_suffix("::self")
            .map(str::to_string)
            .unwrap_or(full)
    };
    let last_segment = |path: &str| path.rsplit("::").next().unwrap_or(path).to_string();

    match node.kind() {
        "use_list" => {
            for child in node.named_children(&mut node.walk()) {
                collect_rust_use(child, source, prefix, imports);
            }
        }
        "scoped_use_list" => {
            let prefix = join(node.child_by_field_name("path").and_then(text));
            if let Some(list) = node.child_by_field_name("list") {
                collect_rust_use(list, source, &prefix, imports);
            }
        }
        "use_wildcard" => {
            let base = join(node.named_child(0).and_then(text));
            imports.push(ImportSymbol {
                local_name: "*".to_string(),
                module_path: if base.is_empty() {
                    "*".to_string()
                } else {
                    format!("{base}::*")
                },
                export_name: None,
            });
        }
        "use_as_clause" => {
            let module_path = join(node.child_by_field_name("path").and_then(text));
            let Some(alias) = node.child_by_field_name("alias").and_then(text) else {
                return;
            };
            if module_path.is_empty() {
                return;
            }
            imports.push(ImportSymbol {
                local_name: alias,
                export_name: Some(last_segment(&module_path)),
                module_path,
            });
        }
        // `identifier`, `scoped_identifier`, `crate`, `self`, `super`, …
        _ => {
            let module_path = join(text(node));
            if module_path.is_empty() {
                return;
            }
            imports.push(ImportSymbol {
                local_name: last_segment(&module_path),
                module_path,
                export_name: None,
            });
        }
    }
}

/// Imports of a Python `import_statement` / `import_from_statement`, one per
/// (repeated) `name` field:
/// - `import a.b, c as d` → modules `a.b` (local `a.b`) and `c` (local `d`);
/// - `from m import A, B as C` → module `m`, `export=A` / `export=B`;
/// - `from m import *` → local `*`.
fn python_import_symbols(node: &TsNode, source: &str) -> Vec<ImportSymbol> {
    let text = |n: TsNode| {
        n.utf8_text(source.as_bytes())
            .ok()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    // `(name, alias)` of a `dotted_name` / `aliased_import`.
    let name_and_alias = |n: TsNode| -> Option<(String, Option<String>)> {
        if n.kind() == "aliased_import" {
            let name = n.child_by_field_name("name").and_then(text)?;
            Some((name, n.child_by_field_name("alias").and_then(text)))
        } else {
            Some((text(n)?, None))
        }
    };
    let names: Vec<TsNode> = node
        .children_by_field_name("name", &mut node.walk())
        .collect();

    if node.kind() == "import_statement" {
        return names
            .into_iter()
            .filter_map(name_and_alias)
            .map(|(module_path, alias)| ImportSymbol {
                local_name: alias.unwrap_or_else(|| module_path.clone()),
                module_path,
                export_name: None,
            })
            .collect();
    }

    let Some(module_path) = node.child_by_field_name("module_name").and_then(text) else {
        return Vec::new();
    };
    if child_of_kind(node, "wildcard_import").is_some() {
        return vec![ImportSymbol {
            local_name: "*".to_string(),
            module_path,
            export_name: None,
        }];
    }
    names
        .into_iter()
        .filter_map(name_and_alias)
        .map(|(name, alias)| ImportSymbol {
            local_name: alias.unwrap_or_else(|| name.clone()),
            module_path: module_path.clone(),
            export_name: Some(name),
        })
        .collect()
}

/// Import of a C# `using_directive`: `using A.B;`, `using static A.B;`,
/// `global using A.B;` and aliases `using X = A.B<T>;`. The `name` field is
/// the alias; the imported namespace/type is the remaining name child.
fn csharp_using_symbols(node: &TsNode, source: &str) -> Option<ImportSymbol> {
    let text = |n: TsNode| n.utf8_text(source.as_bytes()).ok().map(str::to_string);
    let alias = node.child_by_field_name("name");
    let target = node.named_children(&mut node.walk()).find(|c| {
        Some(*c) != alias
            && matches!(
                c.kind(),
                "identifier" | "qualified_name" | "generic_name" | "alias_qualified_name"
            )
    })?;
    let module_path = text(target)?;
    // Last segment without type arguments: `A.B.List<int>` → `List`.
    let last_part = module_path
        .split('<')
        .next()
        .unwrap_or(&module_path)
        .rsplit(['.', ':'])
        .next()
        .unwrap_or(&module_path)
        .to_string();
    if last_part.is_empty() {
        return None;
    }
    Some(ImportSymbol {
        local_name: alias.and_then(text).unwrap_or_else(|| last_part.clone()),
        module_path,
        export_name: Some(last_part),
    })
}

/// Imports of a Go `import_declaration`: a single `import_spec` or an
/// `import_spec_list`. Each spec binds a package (not a symbol) under its
/// optional `name` (alias, `_` or `.`), else the last path segment.
/// Path of a Ruby `require 'x'` / `require_relative 'x'` call with a
/// literal path and no receiver. `require_relative` paths are returned
/// relative to the file (`./x`, `../a/x`), so the resolver anchors them.
fn ruby_require_path(node: &TsNode, source: &str) -> Option<String> {
    if node.kind() != "call" || node.child_by_field_name("receiver").is_some() {
        return None;
    }
    let method = node
        .child_by_field_name("method")?
        .utf8_text(source.as_bytes())
        .ok()?;
    if !matches!(method, "require" | "require_relative") {
        return None;
    }
    let arguments = node.child_by_field_name("arguments")?;
    if arguments.named_child_count() != 1 {
        return None;
    }
    let string = arguments
        .named_child(0)
        .filter(|arg| arg.kind() == "string")?;
    // Interpolated strings (`"#{dir}/x"`) have more than one part.
    if string.named_child_count() != 1 {
        return None;
    }
    let path = string
        .named_child(0)
        .filter(|part| part.kind() == "string_content")?
        .utf8_text(source.as_bytes())
        .ok()?
        .trim();
    if path.is_empty() {
        None
    } else if method == "require_relative" && !path.starts_with('.') {
        Some(format!("./{path}"))
    } else {
        Some(path.to_string())
    }
}

/// A Ruby require imports a file; it binds no names (everything the file
/// defines becomes visible), so the import is named after the file.
fn ruby_require_symbols(node: &TsNode, source: &str) -> Vec<ImportSymbol> {
    let Some(module_path) = ruby_require_path(node, source) else {
        return Vec::new();
    };
    let file = module_path.rsplit('/').next().unwrap_or(&module_path);
    let local_name = file.strip_suffix(".rb").unwrap_or(file).to_string();
    vec![ImportSymbol {
        local_name,
        module_path,
        export_name: None,
    }]
}

fn go_import_symbols(node: &TsNode, source: &str) -> Vec<ImportSymbol> {
    let text = |n: TsNode| n.utf8_text(source.as_bytes()).ok().map(str::to_string);
    let specs: Vec<TsNode> = match child_of_kind(node, "import_spec_list") {
        Some(list) => list
            .named_children(&mut list.walk())
            .filter(|c| c.kind() == "import_spec")
            .collect(),
        None => child_of_kind(node, "import_spec").into_iter().collect(),
    };

    specs
        .into_iter()
        .filter_map(|spec| {
            let raw = spec.child_by_field_name("path").and_then(text)?;
            let module_path = raw.trim_matches(['"', '`'].as_ref()).trim().to_string();
            if module_path.is_empty() {
                return None;
            }
            let local_name = spec
                .child_by_field_name("name")
                .and_then(text)
                .unwrap_or_else(|| {
                    module_path
                        .rsplit('/')
                        .next()
                        .unwrap_or(&module_path)
                        .to_string()
                });
            Some(ImportSymbol {
                local_name,
                module_path,
                export_name: None,
            })
        })
        .collect()
}

/// Imports of a PHP `namespace_use_declaration`: `use A\B;`,
/// `use A\B as C, D\E;`, `use function A\f;` and group uses
/// `use A\{B, C as D};` (prefix is a `namespace_name` child of the
/// declaration, clauses sit in the `body` group).
fn php_import_symbols(node: &TsNode, source: &str) -> Vec<ImportSymbol> {
    let text = |n: TsNode| n.utf8_text(source.as_bytes()).ok().map(str::to_string);
    let prefix = child_of_kind(node, "namespace_name").and_then(text);
    let clauses = node.child_by_field_name("body").unwrap_or(*node);

    clauses
        .named_children(&mut clauses.walk())
        .filter(|c| c.kind() == "namespace_use_clause")
        .filter_map(|clause| {
            let path = clause
                .named_children(&mut clause.walk())
                .find(|c| matches!(c.kind(), "qualified_name" | "name"))
                .and_then(text)?;
            let path = path.trim_start_matches('\\');
            let module_path = match &prefix {
                Some(prefix) => format!("{}\\{path}", prefix.trim_start_matches('\\')),
                None => path.to_string(),
            };
            let last_part = module_path
                .rsplit('\\')
                .next()
                .unwrap_or(&module_path)
                .to_string();
            let alias = clause.child_by_field_name("alias").and_then(text);
            Some(ImportSymbol {
                local_name: alias.unwrap_or_else(|| last_part.clone()),
                module_path,
                export_name: Some(last_part),
            })
        })
        .collect()
}

/// Field name holding the module path of an import declaration, if the
/// grammar exposes one (see `import_module_path` for field-less grammars).
///
/// The field must exist in the language's grammar; the grammar guard test
/// (`grammar_guard_tests`) enforces this.
fn import_path_field(language: Language) -> Option<&'static str> {
    match language {
        // Rust `use_declaration` → `argument`; see `rust_use_symbols`.
        Language::Rust => None,
        Language::JavaScript | Language::Jsx | Language::TypeScript | Language::Tsx => {
            Some("source")
        }
        // `import_from_statement` only; see `python_import_symbols`.
        Language::Python => Some("module_name"),
        // Go `import_declaration` has no fields; see `go_import_symbols`.
        Language::Go => None,
        Language::Java => Some("name"),
        Language::C | Language::Cpp => Some("path"),
        // C# `using_directive`'s `name` field is the alias; see `csharp_using_symbols`.
        Language::CSharp => None,
        // PHP `namespace_use_declaration` has no path field; see `php_import_symbols`.
        Language::Php => None,
        // Ruby `require` is a `call`; see `ruby_require_path`.
        Language::Ruby => None,
        // Swift `import_declaration` has no fields; the path is an `identifier` child.
        Language::Swift => None,
        // Kotlin `import` has no fields; the path is a `qualified_identifier` child.
        Language::Kotlin => None,
        _ => Some("source"),
    }
}

fn import_module_path(node: &TsNode, source: &str, language: Language) -> Option<String> {
    let child = if language == Language::Kotlin {
        node.children(&mut node.walk())
            .find(|c| c.kind() == "qualified_identifier")?
    } else {
        import_path_field(language)
            .and_then(|field| node.child_by_field_name(field))
            .or_else(|| {
                // Fallback: get first string-like child
                node.children(&mut node.walk())
                    .find(|c| matches!(c.kind(), "string" | "identifier" | "scoped_identifier"))
            })?
    };

    let raw = child.utf8_text(source.as_bytes()).ok()?.trim().to_string();
    let trimmed = raw
        .trim_matches(['"', '\'', '`'].as_ref())
        .trim()
        .to_string();

    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn collect_import_symbols(
    node: TsNode,
    source: &str,
    module_path: &str,
    imports: &mut Vec<ImportSymbol>,
) {
    for child in node.children(&mut node.walk()) {
        match child.kind() {
            "identifier" => {
                if let Ok(text) = child.utf8_text(source.as_bytes()) {
                    imports.push(ImportSymbol {
                        local_name: text.to_string(),
                        module_path: module_path.to_string(),
                        export_name: None,
                    });
                }
            }
            // `* as name`: no fields, the name is the `identifier` child.
            "namespace_import" => {
                let name = child_of_kind(&child, "identifier")
                    .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                    .map(|s| s.to_string());
                if let Some(name) = name {
                    imports.push(ImportSymbol {
                        local_name: name,
                        module_path: module_path.to_string(),
                        export_name: None,
                    });
                }
            }
            "named_imports" => collect_named_imports(child, source, module_path, imports),
            "import_specifier" => collect_import_specifier(child, source, module_path, imports),
            _ => {}
        }
    }
}

fn collect_named_imports(
    node: TsNode,
    source: &str,
    module_path: &str,
    imports: &mut Vec<ImportSymbol>,
) {
    for child in node.children(&mut node.walk()) {
        if child.kind() == "import_specifier" {
            collect_import_specifier(child, source, module_path, imports);
        }
    }
}

fn collect_import_specifier(
    node: TsNode,
    source: &str,
    module_path: &str,
    imports: &mut Vec<ImportSymbol>,
) {
    let export_name = node
        .child_by_field_name("name")
        .and_then(|n| n.utf8_text(source.as_bytes()).ok())
        .map(|s| s.to_string());
    let alias = node
        .child_by_field_name("alias")
        .and_then(|n| n.utf8_text(source.as_bytes()).ok())
        .map(|s| s.to_string());

    if let Some(export_name) = export_name {
        let local_name = alias.clone().unwrap_or_else(|| export_name.clone());
        imports.push(ImportSymbol {
            local_name,
            module_path: module_path.to_string(),
            export_name: Some(export_name),
        });
    }
}

fn add_module_node(
    node: &TsNode,
    source: &str,
    project_root: &Path,
    language: Language,
    file_path: &str,
    parent_id: String,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
    now_ms: i64,
) {
    let Some(name) = module_name(node, source, language) else {
        return;
    };

    let start = node.start_position();
    let end = node.end_position();
    let qualified_name = format!("{}::{}", file_path, name);
    let id = node_id_for_symbol(
        file_path,
        "module",
        &qualified_name,
        start.row as i64 + 1,
        start.column as i64,
    );

    let signature = match language {
        Language::Rust => rust_module_target(project_root, file_path, &name),
        _ => None,
    };

    let (visibility, is_exported) = match read_declaration_visibility(*node, language, source) {
        Some((vis, exported)) => (Some(vis), exported),
        None => (None, false),
    };

    nodes.push(Node {
        id: id.clone(),
        kind: NodeKind::Module,
        name,
        qualified_name,
        file_path: file_path.to_string(),
        language,
        start_line: start.row as i64 + 1,
        end_line: end.row as i64 + 1,
        start_column: start.column as i64,
        end_column: end.column as i64,
        docstring: None,
        signature,
        visibility,
        is_exported,
        is_async: false,
        is_static: false,
        is_abstract: false,
        decorators: None,
        type_parameters: None,
        updated_at: now_ms,
        cluster_id: None,
    });

    edges.push(Edge {
        source: parent_id.clone(),
        target: id.clone(),
        kind: EdgeKind::Contains,
        metadata: None,
        line: Some(start.row as i64 + 1),
        column: Some(start.column as i64),
        confidence: 1.0,
        process_id: None,
    });
}

fn module_name(node: &TsNode, source: &str, language: Language) -> Option<String> {
    match language {
        Language::Rust => node
            .child_by_field_name("name")
            .and_then(|n| n.utf8_text(source.as_bytes()).ok())
            .map(|s| s.to_string()),
        // `package a.b.c` → module `a.b.c`
        Language::Kotlin => kotlin_package_name(node, source),
        Language::Java => node
            .children(&mut node.walk())
            .find(|c| matches!(c.kind(), "scoped_identifier" | "identifier"))
            .and_then(|n| n.utf8_text(source.as_bytes()).ok())
            .map(str::to_string),
        _ => None,
    }
}

fn rust_module_target(project_root: &Path, file_path: &str, name: &str) -> Option<String> {
    let base_dir = Path::new(file_path)
        .parent()
        .unwrap_or_else(|| Path::new(""));
    let candidate_file = base_dir.join(format!("{name}.rs"));
    let candidate_mod = base_dir.join(name).join("mod.rs");

    if project_root.join(&candidate_file).is_file() {
        Some(candidate_file.to_string_lossy().to_string())
    } else if project_root.join(&candidate_mod).is_file() {
        Some(candidate_mod.to_string_lossy().to_string())
    } else {
        None
    }
}

fn add_export_nodes(
    node: &TsNode,
    source: &str,
    language: Language,
    file_path: &str,
    parent_id: String,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
    now_ms: i64,
) {
    let exports = export_symbols(node, source, language);
    if exports.is_empty() {
        return;
    }

    let start = node.start_position();
    let end = node.end_position();

    for export in exports {
        let qualified_name = format!("{}::export::{}", file_path, export.name);
        let id = node_id_for_symbol(
            file_path,
            "export",
            &qualified_name,
            start.row as i64 + 1,
            start.column as i64,
        );

        nodes.push(Node {
            id: id.clone(),
            kind: NodeKind::Export,
            name: export.name,
            qualified_name,
            file_path: file_path.to_string(),
            language,
            start_line: start.row as i64 + 1,
            end_line: end.row as i64 + 1,
            start_column: start.column as i64,
            end_column: end.column as i64,
            docstring: None,
            signature: export.module_path,
            visibility: None,
            is_exported: true,
            is_async: false,
            is_static: false,
            is_abstract: false,
            decorators: None,
            type_parameters: None,
            updated_at: now_ms,
            cluster_id: None,
        });

        edges.push(Edge {
            source: parent_id.clone(),
            target: id.clone(),
            kind: EdgeKind::Contains,
            metadata: None,
            line: Some(start.row as i64 + 1),
            column: Some(start.column as i64),
            confidence: 1.0,
            process_id: None,
        });
        edges.push(Edge {
            source: parent_id.clone(),
            target: id,
            kind: EdgeKind::Exports,
            metadata: None,
            line: Some(start.row as i64 + 1),
            column: Some(start.column as i64),
            confidence: 1.0,
            process_id: None,
        });
    }
}

fn export_symbols(node: &TsNode, source: &str, language: Language) -> Vec<ExportSymbol> {
    match language {
        // === JavaScript/TypeScript family ===
        Language::JavaScript | Language::Jsx | Language::TypeScript | Language::Tsx => {
            let module_path = export_module_path(node, source);
            let mut names = Vec::new();
            collect_export_names(*node, source, &mut names);

            if names.is_empty() {
                return Vec::new();
            }

            names
                .into_iter()
                .map(|name| ExportSymbol {
                    name,
                    module_path: module_path.clone(),
                })
                .collect()
        }

        // === Rust ===
        // `pub use` re-exports (wildcards have no single name)
        Language::Rust => rust_use_symbols(node, source)
            .into_iter()
            .filter(|import| import.local_name != "*")
            .map(|import| ExportSymbol {
                name: import.local_name,
                module_path: Some(import.module_path),
            })
            .collect(),

        // === Python: explicit __all__ or all public names ===
        Language::Python => {
            // Python doesn't use explicit export statements; all public names are exported
            // This would be handled at the symbol level during extraction
            Vec::new()
        }

        // === Go: capitalized identifiers are exported ===
        Language::Go => Vec::new(),

        // === Java: public modifier on class declaration ===
        Language::Java => {
            // Java exports are handled via method visibility
            Vec::new()
        }

        // === C/C++: header file declarations ===
        Language::C | Language::Cpp => {
            // C/C++ exports are implicit via header files
            Vec::new()
        }

        // === C#: public modifier ===
        Language::CSharp => Vec::new(),

        // === PHP: global namespace ===
        Language::Php => Vec::new(),

        // === Ruby: implicit (all top-level) ===
        Language::Ruby => Vec::new(),

        // === Swift: public modifier ===
        Language::Swift => Vec::new(),

        // === Kotlin: implicit (all top-level unless private) ===
        Language::Kotlin => kotlin_export_symbol(node, source).into_iter().collect(),

        // Blazor, markup and unsupported languages: no exports
        _ => Vec::new(),
    }
}

/// Implicit export of a top-level Kotlin declaration: every non-`private`
/// declaration, keyed by its fully-qualified name (`package.Name`), which
/// is the module path Kotlin imports refer to.
fn kotlin_export_symbol(node: &TsNode, source: &str) -> Option<ExportSymbol> {
    if !matches!(
        node.kind(),
        "class_declaration"
            | "object_declaration"
            | "function_declaration"
            | "property_declaration"
            | "type_alias"
    ) {
        return None;
    }

    let is_private = child_of_kind(node, "modifiers")
        .and_then(|modifiers| child_of_kind(&modifiers, "visibility_modifier"))
        .is_some_and(|vis| vis.utf8_text(source.as_bytes()) == Ok("private"));
    if is_private {
        return None;
    }

    let name = node_name(node, source, Language::Kotlin)?;
    let package = node
        .parent()
        .and_then(|root| child_of_kind(&root, "package_header"))
        .and_then(|header| kotlin_package_name(&header, source));
    let module_path = package.map_or_else(|| name.clone(), |p| format!("{p}.{name}"));
    Some(ExportSymbol {
        name,
        module_path: Some(module_path),
    })
}

fn export_module_path(node: &TsNode, source: &str) -> Option<String> {
    let child = node.child_by_field_name("source")?;
    let raw = child.utf8_text(source.as_bytes()).ok()?.trim().to_string();
    let trimmed = raw.trim_matches(['"', '\''].as_ref()).to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn collect_export_names(node: TsNode, source: &str, names: &mut Vec<String>) {
    if node.kind() == "export_specifier" {
        let alias = node
            .child_by_field_name("alias")
            .and_then(|n| n.utf8_text(source.as_bytes()).ok())
            .map(|s| s.to_string());
        let name = alias.or_else(|| {
            node.child_by_field_name("name")
                .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                .map(|s| s.to_string())
        });
        if let Some(name) = name {
            names.push(name);
        }
        return;
    }

    if matches!(
        node.kind(),
        "function_declaration"
            | "class_declaration"
            | "interface_declaration"
            | "type_alias_declaration"
            | "enum_declaration"
            | "variable_declarator"
    ) {
        let name = node
            .child_by_field_name("name")
            .and_then(|n| n.utf8_text(source.as_bytes()).ok())
            .map(|s| s.to_string());
        if let Some(name) = name {
            names.push(name);
        }
        return;
    }

    for child in node.children(&mut node.walk()) {
        collect_export_names(child, source, names);
    }
}

fn build_import_signature(module_path: &str, export_name: Option<&str>) -> String {
    match export_name {
        Some(name) => format!("{module_path}|export={name}"),
        None => module_path.to_string(),
    }
}

fn node_key(kind: NodeKind, start: tree_sitter::Point, name: &str) -> String {
    format!("{:?}:{}:{}:{}", kind, start.row, start.column, name)
}

fn is_callable_kind(kind: NodeKind) -> bool {
    matches!(kind, NodeKind::Function | NodeKind::Method)
}

/// Call-expression node kinds per language.
///
/// Every kind listed here must exist in the language's grammar; the grammar
/// guard test (`grammar_guard_tests`) enforces this.
fn call_expression_kinds(language: Language) -> &'static [&'static str] {
    match language {
        // Rust
        Language::Rust => &["call_expression", "macro_invocation"],
        // JavaScript/TypeScript family
        Language::JavaScript | Language::Jsx | Language::TypeScript | Language::Tsx => {
            &["call_expression"]
        }
        // Python
        Language::Python => &["call"],
        // Go
        Language::Go => &["call_expression"],
        // Java: method calls and `new Foo()` (recorded as a call to `Foo`)
        Language::Java => &["method_invocation", "object_creation_expression"],
        // C/C++
        Language::C | Language::Cpp => &["call_expression"],
        // C#
        Language::CSharp => &["invocation_expression"],
        // PHP: `f()`, `$o->m()`, `$o?->m()`, `C::m()` and `new C()` (recorded
        // as a call to `C`)
        Language::Php => &[
            "function_call_expression",
            "member_call_expression",
            "nullsafe_member_call_expression",
            "scoped_call_expression",
            "object_creation_expression",
        ],
        // Ruby: `call` covers `recv.m(...)`, `m(...)` and `Foo.new` (recorded
        // as a call to `Foo`). Bare `m` without receiver or parentheses is an
        // `identifier`, indistinguishable from a local variable, and is skipped.
        Language::Ruby => &["call"],
        // Swift: calls and `Foo<T>()` (recorded as a call to `Foo`)
        Language::Swift => &["call_expression", "constructor_expression"],
        // Kotlin
        Language::Kotlin => &["call_expression"],
        // Markup, Blazor and unsupported languages: no calls
        _ => &[],
    }
}

fn is_call_expression(kind: &str, language: Language) -> bool {
    call_expression_kinds(language).contains(&kind)
}

/// Field names holding the callee of a call expression, tried in order.
///
/// Every field listed here must exist in the language's grammar; the grammar
/// guard test (`grammar_guard_tests`) enforces this.
fn call_name_fields(language: Language) -> &'static [&'static str] {
    match language {
        // `call_expression` → `function`; `macro_invocation` → `macro`
        Language::Rust => &["function", "macro"],
        Language::JavaScript | Language::Jsx | Language::TypeScript | Language::Tsx => {
            &["function"]
        }
        Language::Python => &["function"],
        Language::Go => &["function"],
        // `method_invocation` → `name`; `object_creation_expression` → `type`
        Language::Java => &["name", "type"],
        Language::C | Language::Cpp => &["function"],
        Language::CSharp => &["function"],
        // `function_call_expression` → `function`; member/scoped calls → `name`
        Language::Php => &["function", "name"],
        Language::Ruby => &["method"],
        // Kotlin and Swift `call_expression` have no fields; see
        // `kotlin_callee` / `swift_callee`.
        _ => &[],
    }
}

/// Callee of a Kotlin `call_expression`: its first named child is the callee
/// expression, followed by `type_arguments` / `value_arguments` /
/// `annotated_lambda`. For `a.b.foo()` (`navigation_expression`) the callee
/// is the last `identifier`. Other callee shapes (calls on call results,
/// lambdas, ...) have no usable name.
fn kotlin_callee<'tree>(node: &TsNode<'tree>) -> Option<TsNode<'tree>> {
    let callee = node.named_child(0)?;
    match callee.kind() {
        "identifier" => Some(callee),
        "navigation_expression" => callee
            .named_children(&mut callee.walk())
            .last()
            .filter(|n| n.kind() == "identifier"),
        _ => None,
    }
}

/// Callee of a Swift `call_expression` / `constructor_expression`.
///
/// `call_expression` has no fields: its first named child is the callee
/// (`simple_identifier`, or `navigation_expression` whose `suffix` is a
/// `navigation_suffix` holding the member name), followed by `call_suffix`.
/// `constructor_expression` (`Foo<T>()`) names the type in `constructed_type`.
fn swift_callee<'tree>(node: &TsNode<'tree>) -> Option<TsNode<'tree>> {
    if node.kind() == "constructor_expression" {
        return node
            .child_by_field_name("constructed_type")
            .filter(|t| t.kind() == "user_type")
            .and_then(|t| child_of_kind(&t, "type_identifier"));
    }
    let callee = node.named_child(0)?;
    match callee.kind() {
        "simple_identifier" => Some(callee),
        "navigation_expression" => callee
            .child_by_field_name("suffix")
            .and_then(|suffix| suffix.child_by_field_name("suffix"))
            .filter(|n| n.kind() == "simple_identifier"),
        _ => None,
    }
}

/// Callee of a Ruby `call`: its `method`, except `Foo.new` / `A::Foo.new`,
/// which is recorded as a call to the class `Foo`.
fn ruby_callee<'tree>(node: &TsNode<'tree>, source: &str) -> Option<TsNode<'tree>> {
    let method = node.child_by_field_name("method")?;
    if method.utf8_text(source.as_bytes()) == Ok("new") {
        if let Some(receiver) = node.child_by_field_name("receiver") {
            match receiver.kind() {
                "constant" => return Some(receiver),
                "scope_resolution" => return receiver.child_by_field_name("name"),
                _ => {}
            }
        }
    }
    Some(method)
}

/// Callee of a Go `call_expression`: the `function` expression unwrapped to
/// its name (`f`, `pkg.F` / `x.m` → field, `F[T]`, `(f)`). Calls of func
/// literals or call results (`func() {…}()`, `f()()`) have no name.
fn go_callee<'tree>(node: &TsNode<'tree>) -> Option<TsNode<'tree>> {
    let mut current = node.child_by_field_name("function")?;
    loop {
        current = match current.kind() {
            "identifier" | "field_identifier" | "type_identifier" => return Some(current),
            "selector_expression" => current.child_by_field_name("field")?,
            "qualified_type" => current.child_by_field_name("name")?,
            "index_expression" => current.child_by_field_name("operand")?,
            "type_instantiation_expression" => current.child_by_field_name("type")?,
            "parenthesized_expression" => current.named_child(0)?,
            _ => return None,
        };
    }
}

/// Callee of a PHP call. Namespaced names (`\App\fmt()`, `new \App\Foo()`)
/// are reduced to their last segment; `new` has no fields, the class is a
/// `name` / `qualified_name` child.
fn php_callee<'tree>(node: &TsNode<'tree>) -> Option<TsNode<'tree>> {
    let callee = if node.kind() == "object_creation_expression" {
        node.named_children(&mut node.walk())
            .find(|c| matches!(c.kind(), "name" | "qualified_name"))?
    } else {
        call_name_fields(Language::Php)
            .iter()
            .find_map(|field| node.child_by_field_name(field))?
    };
    match callee.kind() {
        "name" => Some(callee),
        "qualified_name" => child_of_kind(&callee, "name"),
        _ => None,
    }
}

fn call_name(node: &TsNode, source: &str, language: Language) -> Option<String> {
    let mut callee = match language {
        Language::Kotlin => kotlin_callee(node)?,
        Language::Swift => swift_callee(node)?,
        Language::Ruby => ruby_callee(node, source)?,
        Language::C | Language::Cpp => c_callee(node)?,
        Language::Php => php_callee(node)?,
        Language::Go => go_callee(node)?,
        _ => call_name_fields(language)
            .iter()
            .find_map(|field| node.child_by_field_name(field))?,
    };
    // `new Foo<T>()`: drop the type arguments.
    if language == Language::Java && callee.kind() == "generic_type" {
        callee = callee.named_child(0)?;
    }

    let raw = callee.utf8_text(source.as_bytes()).ok()?.to_string();
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let name = trimmed
        .rsplit("::")
        .next()
        .unwrap_or(trimmed)
        .rsplit('.')
        .next()
        .unwrap_or(trimmed)
        .rsplit("->")
        .next()
        .unwrap_or(trimmed)
        .to_string();

    if name.is_empty() { None } else { Some(name) }
}

/// Receiver / qualifier of a call when it is a plain (possibly dotted) name:
/// `Report` in `Report.describe()`, `a` in `a.Describe()`, `crate::util` in
/// `crate::util::f()`, `requests` in `requests.post()`. The resolver matches
/// it against import names. Expression receivers (`f().g()`, `$this->m()`)
/// yield `None`.
fn call_qualifier(node: &TsNode, source: &str, language: Language) -> Option<String> {
    let receiver = match language {
        Language::Java => node.child_by_field_name("object")?,
        Language::Php => node
            .child_by_field_name("object")
            .or_else(|| node.child_by_field_name("scope"))?,
        Language::Ruby => node.child_by_field_name("receiver")?,
        Language::Go => node
            .child_by_field_name("function")
            .filter(|f| f.kind() == "selector_expression")?
            .child_by_field_name("operand")?,
        Language::Kotlin => node
            .named_child(0)
            .filter(|callee| callee.kind() == "navigation_expression")?
            .named_child(0)?,
        Language::Swift => node
            .named_child(0)
            .filter(|callee| callee.kind() == "navigation_expression")?
            .child_by_field_name("target")?,
        // Rust, JS/TS, Python, C/C++, C#: the callee is `qualifier<sep>name`.
        _ => {
            let callee = call_name_fields(language)
                .iter()
                .find_map(|field| node.child_by_field_name(field))?;
            let text = callee.utf8_text(source.as_bytes()).ok()?.trim();
            let end = ["::", ".", "->"]
                .iter()
                .filter_map(|sep| text.rfind(sep))
                .max()?;
            return plain_qualifier(text.get(..end)?);
        }
    };
    plain_qualifier(receiver.utf8_text(source.as_bytes()).ok()?)
}

fn plain_qualifier(text: &str) -> Option<String> {
    let text = text.trim();
    let plain = !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | ':' | '\\'));
    plain.then(|| text.to_string())
}

/// Declaration node kinds per language: `(tree-sitter kind, NodeKind, is_container)`.
///
/// Every kind listed here must exist in the language's grammar; the grammar
/// guard test (`grammar_guard_tests`) enforces this.
fn node_kind_mappings(language: Language) -> &'static [(&'static str, NodeKind, bool)] {
    match language {
        // === Rust ===
        Language::Rust => &[
            ("function_item", NodeKind::Function, false),
            ("struct_item", NodeKind::Struct, true),
            ("enum_item", NodeKind::Enum, true),
            ("trait_item", NodeKind::Trait, true),
            ("use_declaration", NodeKind::Import, false),
            // `pub use` is also emitted as an export; see `walk_tree_collect`.
            ("mod_item", NodeKind::Module, true),
        ],

        // === JavaScript/TypeScript family ===
        Language::JavaScript | Language::Jsx | Language::TypeScript | Language::Tsx => &[
            ("function_declaration", NodeKind::Function, false),
            // Named after their variable; see `js_function_value_name`.
            ("arrow_function", NodeKind::Function, false),
            ("function_expression", NodeKind::Function, false),
            ("class_declaration", NodeKind::Class, true),
            ("method_definition", NodeKind::Method, false),
            ("interface_declaration", NodeKind::Interface, true),
            ("type_alias_declaration", NodeKind::TypeAlias, false),
            ("import_statement", NodeKind::Import, false),
            ("export_statement", NodeKind::Export, false),
            ("enum_declaration", NodeKind::Enum, true),
            ("variable_declarator", NodeKind::Variable, false),
        ],

        // === Python ===
        Language::Python => &[
            ("function_definition", NodeKind::Function, false),
            ("class_definition", NodeKind::Class, true),
            ("decorated_definition", NodeKind::Function, false),
            ("import_statement", NodeKind::Import, false),
            ("import_from_statement", NodeKind::Import, false),
            ("assignment", NodeKind::Variable, false),
            ("augmented_assignment", NodeKind::Variable, false),
            ("for_statement", NodeKind::Variable, false),
            ("with_statement", NodeKind::Variable, false),
        ],

        // === Go ===
        Language::Go => &[
            ("function_declaration", NodeKind::Function, false),
            ("method_declaration", NodeKind::Method, false),
            ("type_declaration", NodeKind::Struct, true),
            ("const_declaration", NodeKind::Constant, false),
            ("var_declaration", NodeKind::Variable, false),
            ("import_declaration", NodeKind::Import, false),
            ("type_spec", NodeKind::TypeAlias, false),
            ("interface_type", NodeKind::Interface, true),
            ("struct_type", NodeKind::Struct, true),
        ],

        // === Java ===
        Language::Java => &[
            ("method_declaration", NodeKind::Method, false),
            ("constructor_declaration", NodeKind::Method, false),
            ("class_declaration", NodeKind::Class, true),
            ("interface_declaration", NodeKind::Interface, true),
            ("enum_declaration", NodeKind::Enum, true),
            ("field_declaration", NodeKind::Field, false),
            ("import_declaration", NodeKind::Import, false),
            ("package_declaration", NodeKind::Module, true),
            ("annotation_type_declaration", NodeKind::Interface, true),
        ],

        // === C ===
        Language::C => &[
            ("function_definition", NodeKind::Function, false),
            ("declaration", NodeKind::Variable, false),
            ("struct_specifier", NodeKind::Struct, true),
            ("union_specifier", NodeKind::Struct, true),
            ("enum_specifier", NodeKind::Enum, true),
            ("type_definition", NodeKind::TypeAlias, false),
            ("preproc_include", NodeKind::Import, false),
            ("preproc_def", NodeKind::Constant, false),
            // Function-like macros: callable like functions.
            ("preproc_function_def", NodeKind::Function, false),
        ],

        // === C++ ===
        Language::Cpp => &[
            // Methods are `function_definition`s too; see `cpp_is_method`.
            ("function_definition", NodeKind::Function, false),
            ("class_specifier", NodeKind::Class, true),
            ("struct_specifier", NodeKind::Struct, true),
            ("union_specifier", NodeKind::Struct, true),
            ("enum_specifier", NodeKind::Enum, true),
            ("namespace_definition", NodeKind::Namespace, true),
            ("declaration", NodeKind::Variable, false),
            ("preproc_include", NodeKind::Import, false),
            ("preproc_def", NodeKind::Constant, false),
            // Function-like macros: callable like functions.
            ("preproc_function_def", NodeKind::Function, false),
        ],

        // === C# ===
        Language::CSharp => &[
            ("method_declaration", NodeKind::Method, false),
            ("class_declaration", NodeKind::Class, true),
            ("interface_declaration", NodeKind::Interface, true),
            ("struct_declaration", NodeKind::Struct, true),
            ("enum_declaration", NodeKind::Enum, true),
            ("field_declaration", NodeKind::Field, false),
            ("property_declaration", NodeKind::Property, false),
            ("namespace_declaration", NodeKind::Namespace, true),
            // `namespace A.B;`: the declarations are its siblings.
            (
                "file_scoped_namespace_declaration",
                NodeKind::Namespace,
                false,
            ),
            ("using_directive", NodeKind::Import, false),
        ],

        // === PHP ===
        Language::Php => &[
            ("function_definition", NodeKind::Function, false),
            ("method_declaration", NodeKind::Method, false),
            ("class_declaration", NodeKind::Class, true),
            ("interface_declaration", NodeKind::Interface, true),
            ("trait_declaration", NodeKind::Trait, true),
            ("namespace_definition", NodeKind::Namespace, true),
            ("property_declaration", NodeKind::Property, false),
            ("const_declaration", NodeKind::Constant, false),
            // `use A\B;` (`use_declaration` is a trait use inside a class)
            ("namespace_use_declaration", NodeKind::Import, false),
        ],

        // === Ruby ===
        Language::Ruby => &[
            ("method", NodeKind::Method, false),
            ("singleton_method", NodeKind::Method, false),
            ("class", NodeKind::Class, true),
            ("module", NodeKind::Namespace, true),
            ("assignment", NodeKind::Variable, false),
            ("begin", NodeKind::Variable, false),
        ],

        // === Swift ===
        Language::Swift => &[
            ("function_declaration", NodeKind::Function, false),
            ("init_declaration", NodeKind::Method, false),
            ("deinit_declaration", NodeKind::Method, false),
            // class / struct / enum / extension / actor; see `swift_class_kind`
            ("class_declaration", NodeKind::Class, true),
            ("protocol_declaration", NodeKind::Protocol, true),
            ("property_declaration", NodeKind::Property, false),
            ("import_declaration", NodeKind::Import, false),
        ],

        // === Kotlin ===
        Language::Kotlin => &[
            ("function_declaration", NodeKind::Function, false),
            ("property_declaration", NodeKind::Property, false),
            ("class_declaration", NodeKind::Class, true),
            ("object_declaration", NodeKind::Class, true),
            ("enum_entry", NodeKind::EnumMember, false),
            ("secondary_constructor", NodeKind::Method, false),
            ("companion_object", NodeKind::Class, true),
            ("type_alias", NodeKind::TypeAlias, false),
            ("package_header", NodeKind::Module, false),
            ("import", NodeKind::Import, false),
        ],

        // Markup and unsupported languages: no extraction
        _ => &[],
    }
}

fn map_node_kind(kind: &str, language: Language) -> (Option<NodeKind>, bool) {
    node_kind_mappings(language)
        .iter()
        .find(|(k, _, _)| *k == kind)
        .map_or((None, false), |&(_, node_kind, container)| {
            (Some(node_kind), container)
        })
}

/// Map a tree-sitter node to a `NodeKind`, refining the kind-only mapping
/// with node context where the grammar shares one node kind between several
/// declaration kinds.
fn node_kind(node: &TsNode, source: &str, language: Language) -> (Option<NodeKind>, bool) {
    let mapped = map_node_kind(node.kind(), language);
    match (language, mapped) {
        (Language::Kotlin, (Some(NodeKind::Class), container))
            if node.kind() == "class_declaration" =>
        {
            (Some(kotlin_class_kind(node, source)), container)
        }
        (Language::Swift, (Some(NodeKind::Class), container))
            if node.kind() == "class_declaration" =>
        {
            (Some(swift_class_kind(node)), container)
        }
        (Language::Cpp, (Some(NodeKind::Function), container))
            if node.kind() == "function_definition" && cpp_is_method(node) =>
        {
            (Some(NodeKind::Method), container)
        }
        (Language::Ruby, _) if ruby_require_path(node, source).is_some() => {
            (Some(NodeKind::Import), false)
        }
        _ => mapped,
    }
}

/// Swift `class_declaration` covers `class`, `struct`, `enum`, `extension`
/// and `actor`, distinguished by its `declaration_kind` keyword. Extensions
/// and actors are kept as classes.
fn swift_class_kind(node: &TsNode) -> NodeKind {
    match node
        .child_by_field_name("declaration_kind")
        .map(|k| k.kind())
    {
        Some("struct") => NodeKind::Struct,
        Some("enum") => NodeKind::Enum,
        _ => NodeKind::Class,
    }
}

/// Kotlin `class_declaration` covers classes, interfaces (`interface`
/// keyword child) and enums (`enum` class modifier).
fn kotlin_class_kind(node: &TsNode, source: &str) -> NodeKind {
    if child_of_kind(node, "interface").is_some() {
        return NodeKind::Interface;
    }
    let is_enum = child_of_kind(node, "modifiers").is_some_and(|modifiers| {
        modifiers
            .children(&mut modifiers.walk())
            .filter(|m| m.kind() == "class_modifier")
            .any(|m| m.utf8_text(source.as_bytes()) == Ok("enum"))
    });
    if is_enum {
        NodeKind::Enum
    } else {
        NodeKind::Class
    }
}

fn scan_directory(
    root_dir: &Path,
    config: &CodeGraphConfig,
    mut on_progress: impl FnMut(usize, &str),
) -> Vec<String> {
    let mut files = Vec::new();
    let mut count = 0;

    let mut stack = vec![root_dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let rel_path = match path.strip_prefix(root_dir) {
                Ok(rel) => rel,
                Err(_) => continue,
            };
            let rel_str = rel_path.to_string_lossy().to_string();

            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                let dir_pattern = format!("{}/", rel_str);
                if config.exclude.iter().any(|p| matches_glob(&dir_pattern, p)) {
                    continue;
                }
                // Skip Python virtual environments by their canonical marker file
                // (covers any venv name, not just common patterns like .venv/venv/env)
                if path.join("pyvenv.cfg").exists() {
                    continue;
                }
                stack.push(path);
            } else if entry.file_type().is_ok_and(|t| t.is_file()) {
                if should_include_file(&rel_str, config) {
                    files.push(rel_str.clone());
                    count += 1;
                    on_progress(count, &rel_str);
                }
            }
        }
    }

    files
}

fn should_include_file(file_path: &str, config: &CodeGraphConfig) -> bool {
    for pattern in &config.exclude {
        if matches_glob(file_path, pattern) {
            return false;
        }
    }

    for pattern in &config.include {
        if matches_glob(file_path, pattern) {
            return true;
        }
    }

    false
}

fn matches_glob(file_path: &str, pattern: &str) -> bool {
    globset::Glob::new(pattern)
        .ok()
        .and_then(|glob| glob.compile_matcher().is_match(file_path).then_some(true))
        .unwrap_or(false)
}

fn detect_language(path: &str) -> Language {
    let ext = Path::new(path)
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    match ext.as_str() {
        "ts" => Language::TypeScript,
        "tsx" => Language::Tsx,
        "js" => Language::JavaScript,
        "jsx" => Language::Jsx,
        "py" => Language::Python,
        "go" => Language::Go,
        "rs" => Language::Rust,
        "java" => Language::Java,
        "c" => Language::C,
        "h" => Language::C,
        "cpp" | "cc" | "cxx" | "hpp" => Language::Cpp,
        "cs" => Language::CSharp,
        "php" => Language::Php,
        "rb" => Language::Ruby,
        "swift" => Language::Swift,
        "kt" | "kts" => Language::Kotlin,
        "liquid" => Language::Liquid,
        "razor" | "cshtml" => Language::Blazor,
        // New languages
        "sh" | "bash" => Language::Bash,
        "dart" => Language::Dart,
        "ex" | "exs" => Language::Elixir,
        "elm" => Language::Elm,
        "erl" | "hrl" => Language::Erlang,
        "f" | "f90" | "f95" => Language::Fortran,
        "groovy" | "gradle" => Language::Groovy,
        "hs" => Language::Haskell,
        "jl" => Language::Julia,
        "lua" => Language::Lua,
        "md" | "markdown" => Language::Markdown,
        "m" => Language::Matlab,
        "nix" => Language::Nix,
        "pl" | "pm" => Language::Perl,
        "ps1" => Language::Powershell,
        "r" => Language::R,
        "scala" | "sc" => Language::Scala,
        "toml" => Language::Toml,
        "yml" | "yaml" => Language::Yaml,
        "zig" => Language::Zig,
        _ => Language::Unknown,
    }
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

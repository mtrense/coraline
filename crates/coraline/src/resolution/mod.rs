#![deny(unsafe_code)]

pub mod frameworks;
mod import_path;

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::db;
use crate::types::Node;
use crate::types::{Edge, EdgeKind, Language, NodeKind};

#[derive(Debug, Default)]
pub struct ReferenceResolver;

#[derive(Debug, Clone)]
pub struct ResolveResult {
    pub scanned: usize,
    pub resolved: usize,
    pub remaining: usize,
}

impl ReferenceResolver {
    pub fn resolve_unresolved(
        conn: &mut rusqlite::Connection,
        project_root: &Path,
        limit: usize,
    ) -> std::io::Result<ResolveResult> {
        let unresolved = db::list_unresolved_refs(conn, limit)?;
        if unresolved.is_empty() {
            return Ok(ResolveResult {
                scanned: 0,
                resolved: 0,
                remaining: 0,
            });
        }

        let mut resolved_edges = Vec::new();
        let mut resolved_ids = Vec::new();

        let mut imports_by_file: HashMap<String, Vec<FileImport>> = HashMap::new();

        for row in &unresolved {
            let reference = &row.reference;
            let from_node = db::get_node_by_id(conn, &reference.from_node_id)?;
            let imports: &[FileImport] = match &from_node {
                Some(node) => {
                    if !imports_by_file.contains_key(&node.file_path) {
                        let imports = file_imports(conn, &node.file_path)?;
                        imports_by_file.insert(node.file_path.clone(), imports);
                    }
                    imports_by_file
                        .get(&node.file_path)
                        .map_or(&[], Vec::as_slice)
                }
                None => &[],
            };
            let context = ImportContext::new(
                imports,
                &reference.reference_name,
                reference.qualifier.as_deref(),
            );
            // `use a::b as c; c()` / `import { b as c }`: look up the
            // original name.
            let lookup_name = context
                .symbol
                .and_then(|import| import.export_name.as_deref())
                .unwrap_or(&reference.reference_name);
            let candidates = match reference.reference_kind {
                EdgeKind::Calls => {
                    // Prefer extractor-provided candidate IDs for better locality/precision.
                    let from_ids = reference
                        .candidates
                        .as_ref()
                        .map_or_else(Vec::new, |ids| nodes_from_ids(conn, ids));
                    if from_ids.is_empty() {
                        filter_by_call_kind(db::find_nodes_by_name(conn, lookup_name)?)
                    } else {
                        filter_by_call_kind(from_ids)
                    }
                }
                _ => db::find_nodes_by_name(conn, &reference.reference_name)?,
            };

            let candidates = rank_candidates(
                conn,
                candidates,
                from_node.as_ref(),
                &context,
                &reference.reference_name,
                reference.reference_kind,
            )?;

            // If generic resolution found nothing, try framework-specific hints.
            let candidates = if candidates.is_empty() {
                from_node.as_ref().map_or_else(Vec::new, |from| {
                    framework_fallback(conn, project_root, from, &reference.reference_name)
                })
            } else {
                candidates
            };

            if let [target] = candidates.as_slice() {
                let confidence = confidence_for_reference(&reference.reference_name);
                resolved_edges.push(Edge {
                    source: reference.from_node_id.clone(),
                    target: target.id.clone(),
                    kind: reference.reference_kind,
                    metadata: None,
                    line: Some(reference.line),
                    column: Some(reference.column),
                    confidence,
                    process_id: None,
                });
                resolved_ids.push(row.id);
            }
        }

        if !resolved_edges.is_empty() {
            db::insert_edges(conn, &resolved_edges)?;
        }
        if !resolved_ids.is_empty() {
            db::delete_unresolved_refs(conn, &resolved_ids)?;
        }

        let remaining = unresolved.len().saturating_sub(resolved_ids.len());
        Ok(ResolveResult {
            scanned: unresolved.len(),
            resolved: resolved_ids.len(),
            remaining,
        })
    }
}

/// Compute the resolution-confidence score for a single resolved reference.
///
/// The thresholds mirror the Phase 5.2 spec (borrows `GitNexus`'s per-edge
/// confidence field, used in their `WHERE r.confidence > 0.8` Cypher
/// filters):
///
/// - `0.95` — strongly-typed Rust path (`crate::`, `super::`, `self::`).
/// - `0.5`  — generic name match / framework fallback / heuristic ranker
///   (the default for everything else).
///
/// Alias-chain detection (the `0.7` tier in the spec) would need access to
/// the originating `use ... as ...` AST node, which isn't currently
/// surfaced through [`UnresolvedReference`]. Once it is, this function
/// is the single place to extend.
fn confidence_for_reference(name: &str) -> f32 {
    if name.starts_with("crate::") || name.starts_with("super::") || name.starts_with("self::") {
        0.95
    } else {
        0.5
    }
}

fn nodes_from_ids(conn: &rusqlite::Connection, ids: &[String]) -> Vec<Node> {
    let mut nodes = Vec::new();
    let mut seen = HashSet::new();
    for id in ids {
        if !seen.insert(id) {
            continue;
        }

        if let Ok(Some(node)) = db::get_node_by_id(conn, id) {
            nodes.push(node);
        }
    }
    nodes
}

/// Use framework-specific resolvers to find candidates when name search fails.
fn framework_fallback(
    conn: &rusqlite::Connection,
    project_root: &Path,
    from_node: &Node,
    reference_name: &str,
) -> Vec<Node> {
    // from_node.file_path is relative to the project root
    let from_abs = project_root.join(&from_node.file_path);
    let from_abs_str = from_abs.to_string_lossy();

    let hints = frameworks::framework_path_hints(project_root, &from_abs_str, reference_name);
    if hints.is_empty() {
        return Vec::new();
    }

    // The last "::" segment is the symbol name we're looking for in those files
    let sym_name = reference_name.split("::").last().unwrap_or(reference_name);

    let mut candidates = Vec::new();
    for hint_path in &hints {
        let rel = relative_to_root(hint_path, project_root);
        if let Ok(mut nodes) = db::get_nodes_by_file(conn, &rel, None) {
            nodes.retain(|n| n.name == sym_name || n.name == reference_name);
            candidates.extend(nodes);
        }
    }
    candidates
}

fn relative_to_root(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

fn filter_by_call_kind(nodes: Vec<Node>) -> Vec<Node> {
    let mut seen = HashSet::new();
    let mut filtered = Vec::new();
    for node in nodes {
        if matches!(node.kind, NodeKind::Function | NodeKind::Method)
            && seen.insert(node.id.clone())
        {
            filtered.push(node);
        }
    }
    filtered
}

fn rank_candidates(
    conn: &rusqlite::Connection,
    nodes: Vec<Node>,
    from_node: Option<&Node>,
    context: &ImportContext<'_>,
    symbol_name: &str,
    reference_kind: EdgeKind,
) -> std::io::Result<Vec<Node>> {
    let Some(from_node) = from_node else {
        return Ok(nodes);
    };

    if let Some(import) = context.symbol {
        let export_name = import.export_name.as_deref().unwrap_or(symbol_name);
        if let Some(exports) = export_candidates(conn, &import.module_path, export_name)? {
            return Ok(exports);
        }
    }

    // `Report.describe()` / `a.Describe()` with `Report` / `a` imported: the
    // callee lives in that import's module, or outside the project.
    if !context.qualifier.is_empty() {
        let targets: Vec<_> = context
            .qualifier
            .iter()
            .flat_map(|import| import.targets(import.item_name()))
            .collect();
        return Ok(best_import_matches(nodes, &targets).0);
    }

    let symbol_targets = context.symbol.map_or_else(Vec::new, |import| {
        import.targets(Some(import.export_name.as_deref().unwrap_or(symbol_name)))
    });
    let (import_matches, nodes) = best_import_matches(nodes, &symbol_targets);
    if !import_matches.is_empty() {
        return Ok(import_matches);
    }

    let from_dir = Path::new(&from_node.file_path).parent();
    let mut same_file = Vec::new();
    let mut same_dir = Vec::new();
    let mut others = Vec::new();

    for node in nodes {
        if node.file_path == from_node.file_path {
            same_file.push(node);
        } else if from_dir.is_some() && Path::new(&node.file_path).parent() == from_dir {
            same_dir.push(node);
        } else {
            others.push(node);
        }
    }

    if !same_file.is_empty() {
        Ok(same_file)
    } else if !same_dir.is_empty() {
        Ok(same_dir)
    } else if reference_kind == EdgeKind::Calls {
        // Avoid low-confidence global-name fallback for call edges because
        // it causes noisy cross-project links in mixed active/legacy workspaces.
        Ok(Vec::new())
    } else {
        Ok(others)
    }
}

/// Split `nodes` into the best-scoring matches of `targets` and the rest.
fn best_import_matches(nodes: Vec<Node>, targets: &[ScoredTarget]) -> (Vec<Node>, Vec<Node>) {
    let mut best_score = 0;
    let mut matches: Vec<Node> = Vec::new();
    let mut rest = Vec::new();
    for node in nodes {
        let score = targets
            .iter()
            .filter_map(|target| target.score(&node.file_path))
            .max();
        match score {
            Some(score) if score > best_score => {
                best_score = score;
                rest.append(&mut matches);
                matches.push(node);
            }
            Some(score) if score == best_score => matches.push(node),
            _ => rest.push(node),
        }
    }
    (matches, rest)
}

fn export_candidates(
    conn: &rusqlite::Connection,
    module_path: &str,
    export_name: &str,
) -> std::io::Result<Option<Vec<Node>>> {
    let exports = db::find_exports_by_module(conn, module_path)?;
    if exports.is_empty() {
        return Ok(None);
    }

    let mut exact = Vec::new();
    for export in exports {
        if export.name == export_name {
            exact.push(export);
        }
    }

    if exact.is_empty() {
        Ok(None)
    } else {
        Ok(Some(exact))
    }
}

/// An import node of the calling file.
#[derive(Debug, Clone)]
struct FileImport {
    /// Name the import binds in the file (`Helper`, `a`, `*`).
    local_name: String,
    module_path: String,
    export_name: Option<String>,
    /// Language and file of the import node, for resolving relative paths.
    language: Language,
    file_path: String,
}

impl FileImport {
    fn from_node(node: Node) -> Self {
        let (module_path, export_name) = node
            .signature
            .as_deref()
            .and_then(parse_import_signature)
            .unwrap_or_else(|| (node.name.clone(), None));
        Self {
            local_name: node.name,
            module_path,
            export_name,
            language: node.language,
            file_path: node.file_path,
        }
    }

    /// Imported item name when the module path ends with it (`app.a.Report`,
    /// `crate::a::Circle`). Go imports name packages, never items.
    fn item_name(&self) -> Option<&str> {
        (self.language != Language::Go)
            .then(|| self.export_name.as_deref().unwrap_or(&self.local_name))
    }

    fn targets(&self, item: Option<&str>) -> Vec<ScoredTarget> {
        let package_dir = self.language == Language::Go;
        import_path::import_targets(&self.module_path, self.language, &self.file_path, item)
            .into_iter()
            .map(|target| ScoredTarget {
                target,
                package_dir,
            })
            .collect()
    }

    /// Whether `qualifier` (`Report`, `a.report`, `util::sub`) starts with
    /// the name this import binds.
    fn binds_qualifier(&self, qualifier: &str) -> bool {
        qualifier
            .strip_prefix(self.local_name.as_str())
            .is_some_and(|rest| {
                rest.is_empty()
                    || rest.starts_with('.')
                    || rest.starts_with("::")
                    || rest.starts_with('\\')
            })
    }
}

/// Import target plus whether it names a package directory (Go) rather than
/// a file.
struct ScoredTarget {
    target: import_path::ImportTarget,
    package_dir: bool,
}

impl ScoredTarget {
    fn score(&self, file_path: &str) -> Option<usize> {
        if self.package_dir {
            import_path::dir_match_score(file_path, &self.target)
        } else {
            import_path::match_score(file_path, &self.target)
        }
    }
}

/// Imports of the calling file relevant to one reference.
struct ImportContext<'a> {
    /// Import binding the referenced name itself (`import { describe }`).
    symbol: Option<&'a FileImport>,
    /// Imports binding the call's qualifier (`Report` in `Report.describe()`).
    qualifier: Vec<&'a FileImport>,
}

impl<'a> ImportContext<'a> {
    fn new(imports: &'a [FileImport], name: &str, qualifier: Option<&str>) -> Self {
        Self {
            symbol: imports.iter().find(|import| import.local_name == name),
            qualifier: qualifier.map_or_else(Vec::new, |qualifier| {
                imports
                    .iter()
                    .filter(|import| import.binds_qualifier(qualifier))
                    .collect()
            }),
        }
    }
}

fn file_imports(conn: &rusqlite::Connection, file_path: &str) -> std::io::Result<Vec<FileImport>> {
    Ok(
        db::get_nodes_by_file(conn, file_path, Some(NodeKind::Import))?
            .into_iter()
            .map(FileImport::from_node)
            .collect(),
    )
}

/// `module|export=name` → `(module, Some(name))`; `module` → `(module, None)`.
fn parse_import_signature(signature: &str) -> Option<(String, Option<String>)> {
    if signature.trim().is_empty() {
        return None;
    }

    if let Some((module_path, export_name)) = signature.split_once("|export=") {
        return Some((module_path.to_string(), Some(export_name.to_string())));
    }

    Some((signature.to_string(), None))
}

#[cfg(test)]
mod tests {
    use super::confidence_for_reference;

    #[test]
    fn confidence_is_high_for_strongly_typed_rust_paths() {
        assert!((confidence_for_reference("crate::foo::bar") - 0.95).abs() < f32::EPSILON);
        assert!((confidence_for_reference("super::baz") - 0.95).abs() < f32::EPSILON);
        assert!((confidence_for_reference("self::quux") - 0.95).abs() < f32::EPSILON);
    }

    #[test]
    fn confidence_is_default_for_generic_or_unresolved_paths() {
        assert!((confidence_for_reference("foo") - 0.5).abs() < f32::EPSILON);
        assert!((confidence_for_reference("std::collections::HashMap") - 0.5).abs() < f32::EPSILON);
        assert!((confidence_for_reference("") - 0.5).abs() < f32::EPSILON);
    }
}

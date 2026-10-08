//! Pipeline-wide resolution tests: refs must keep resolving no matter how
//! many unresolvable refs pile up, and edges into a re-indexed file must
//! survive the re-index.
#![allow(clippy::expect_used)]

mod common;

use std::collections::BTreeSet;
use std::path::Path;

use common::{index_project, query_set};
use coraline::resolution::ReferenceResolver;
use coraline::types::{EdgeKind, UnresolvedReference};
use coraline::{config, db, extraction};

/// `caller -> callee` for every stored calls edge.
fn call_edges(project_path: &Path) -> BTreeSet<String> {
    query_set(
        project_path,
        "SELECT s.name || ' -> ' || t.name FROM edges e
           JOIN nodes s ON s.id = e.source JOIN nodes t ON t.id = e.target
          WHERE e.kind = 'calls'",
    )
}

/// `caller -> callee @ file xN` per edge, N = number of identical edges.
const EDGE_ROWS: &str = "SELECT s.name || ' -> ' || t.name || ' @ ' || t.file_path || ' x'
        || (SELECT count(*) FROM edges e2 WHERE e2.source = e.source AND e2.target = e.target
              AND e2.kind = e.kind)
   FROM edges e JOIN nodes s ON s.id = e.source JOIN nodes t ON t.id = e.target";

fn edge_count(project_path: &Path) -> usize {
    let conn = db::open_database(project_path).expect("open db");
    let count: i64 = conn
        .query_row("SELECT count(*) FROM edges", [], |row| row.get(0))
        .expect("count edges");
    usize::try_from(count).expect("non-negative count")
}

fn node_id(project_path: &Path, name: &str) -> String {
    let ids = query_set(
        project_path,
        &format!("SELECT id FROM nodes WHERE name = '{name}' AND kind = 'function'"),
    );
    ids.into_iter().next().expect("node exists")
}

fn call_ref(from: &str, name: &str) -> UnresolvedReference {
    UnresolvedReference {
        from_node_id: from.to_string(),
        reference_name: name.to_string(),
        reference_kind: EdgeKind::Calls,
        line: 1,
        column: 0,
        candidates: None,
        qualifier: None,
    }
}

fn sync(project_path: &Path) {
    let cfg = config::create_default_config(project_path);
    extraction::sync(project_path, &cfg, None).expect("sync");
}

/// G1: more than 10k never-resolvable refs (stdlib calls such as `println`)
/// stored before a resolvable ref must not starve it.
#[test]
fn resolvable_ref_after_many_unresolvable_refs_resolves() {
    let temp = index_project(&[
        ("src/a.ts", "export function target() {}\n"),
        ("src/b.ts", "export function caller() {}\n"),
    ]);
    let path = temp.path();
    let caller = node_id(path, "caller");

    let mut refs: Vec<_> = (0..10_050)
        .map(|i| call_ref(&caller, &format!("missing_{i}")))
        .collect();
    refs.push(call_ref(&caller, "target"));
    let mut conn = db::open_database(path).expect("open db");
    db::insert_unresolved_refs(&mut conn, &refs).expect("insert refs");
    drop(conn);

    sync(path);

    assert!(
        call_edges(path).contains("caller -> target"),
        "later resolvable ref must resolve: {:?}",
        call_edges(path)
    );
}

/// G1 with a small page size: the resolver pages through all refs.
#[test]
fn resolver_pages_through_all_refs() {
    let temp = index_project(&[
        ("src/a.ts", "export function target() {}\n"),
        ("src/b.ts", "export function caller() {}\n"),
    ]);
    let path = temp.path();
    let caller = node_id(path, "caller");

    let mut refs: Vec<_> = (0..7)
        .map(|i| call_ref(&caller, &format!("missing_{i}")))
        .collect();
    refs.push(call_ref(&caller, "target"));
    let mut conn = db::open_database(path).expect("open db");
    db::insert_unresolved_refs(&mut conn, &refs).expect("insert refs");

    let result = ReferenceResolver::resolve_unresolved(&mut conn, path, 2).expect("resolve refs");
    assert_eq!(
        (result.scanned, result.resolved, result.remaining),
        (8, 1, 7)
    );
    assert!(call_edges(path).contains("caller -> target"));
}

fn edit(project_path: &Path, rel: &str, content: &str) {
    std::fs::write(project_path.join(rel), content).expect("write file");
}

fn index_again(project_path: &Path) {
    let cfg = config::create_default_config(project_path);
    extraction::index_all(project_path, &cfg, false, None).expect("index");
}

/// G2: re-indexing a changed file must not drop resolved edges into it from
/// unchanged files (their refs were deleted when they resolved).
#[test]
fn incoming_edges_survive_sync_of_target_file() {
    let temp = index_project(&[
        ("src/a.ts", "export function target() {}\n"),
        ("src/b.ts", "export function caller() { target(); }\n"),
    ]);
    let path = temp.path();
    assert!(call_edges(path).contains("caller -> target"));
    let edges_before = edge_count(path);

    edit(
        path,
        "src/a.ts",
        "// changed\nexport function target() {}\n",
    );
    sync(path);

    assert!(
        call_edges(path).contains("caller -> target"),
        "incoming edge lost: {:?}",
        call_edges(path)
    );
    assert_eq!(edge_count(path), edges_before, "no duplicate edges");
}

/// G2 for an ambiguous call linked to targets in two files: re-indexing one
/// of them re-creates its edge without duplicating the other.
#[test]
fn ambiguous_incoming_edges_are_not_duplicated() {
    let temp = index_project(&[
        ("src/a.ts", "export class A {\n  area() {}\n}\n"),
        ("src/b.ts", "export class B {\n  area() {}\n}\n"),
        ("src/c.ts", "export function caller(s) {\n  s.area();\n}\n"),
    ]);
    let path = temp.path();
    let before = query_set(path, EDGE_ROWS);
    assert_eq!(
        before
            .iter()
            .filter(|e| e.contains("caller -> area"))
            .count(),
        2,
        "{before:?}"
    );
    let count_before = edge_count(path);

    edit(
        path,
        "src/a.ts",
        "// changed\nexport class A {\n  area() {}\n}\n",
    );
    sync(path);

    assert_eq!(query_set(path, EDGE_ROWS), before);
    assert_eq!(edge_count(path), count_before);
}

/// G2 for a call that only resolves through its qualifier (`model.Describe()`
/// → imported package dir).
#[test]
fn incoming_qualified_edges_survive_sync_of_target_file() {
    let temp = index_project(&[
        (
            "svc/app.go",
            "package svc\n\nimport \"example.com/app/internal/model\"\n\nfunc Run() { model.Describe() }\n",
        ),
        (
            "internal/model/model.go",
            "package model\n\nfunc Describe() string { return \"\" }\n",
        ),
    ]);
    let path = temp.path();
    assert!(call_edges(path).contains("Run -> Describe"));

    edit(
        path,
        "internal/model/model.go",
        "package model\n\n// changed\nfunc Describe() string { return \"\" }\n",
    );
    sync(path);

    assert!(
        call_edges(path).contains("Run -> Describe"),
        "incoming edge lost: {:?}",
        call_edges(path)
    );
}

/// G2 through `index_all` (incremental), with an aliased import: the
/// re-queued ref must use the name written at the call site.
#[test]
fn incoming_aliased_edges_survive_incremental_index() {
    let temp = index_project(&[
        ("lib/a.ts", "export function target() {}\n"),
        (
            "app/b.ts",
            "import { target as t } from '../lib/a';\nexport function caller() { t(); }\n",
        ),
    ]);
    let path = temp.path();
    assert!(call_edges(path).contains("caller -> target"));

    edit(
        path,
        "lib/a.ts",
        "// changed\nexport function target() {}\n",
    );
    index_again(path);

    assert!(
        call_edges(path).contains("caller -> target"),
        "incoming edge lost: {:?}",
        call_edges(path)
    );
}

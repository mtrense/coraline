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

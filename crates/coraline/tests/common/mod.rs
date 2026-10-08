//! Shared helpers for per-language extraction fixture tests.
#![allow(dead_code, clippy::expect_used)]

use std::collections::BTreeSet;
use std::path::Path;

use coraline::{config, db, extraction};
use tempfile::TempDir;

/// Index `files` in a fresh project and return the project's temp dir.
pub fn index_project(files: &[(&str, &str)]) -> TempDir {
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let project_path = temp_dir.path();
    db::initialize_database(project_path).expect("Failed to initialize database");

    for (rel, content) in files {
        let path = project_path.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("Failed to create directory");
        }
        std::fs::write(path, content).expect("Failed to write file");
    }

    let cfg = config::create_default_config(project_path);
    extraction::index_all(project_path, &cfg, false, None).expect("Failed to index project");
    temp_dir
}

pub fn query_set(project_path: &Path, sql: &str) -> BTreeSet<String> {
    let conn = db::open_database(project_path).expect("Failed to open database");
    let mut stmt = conn.prepare(sql).expect("Failed to prepare SQL statement");
    stmt.query_map([], |row| row.get::<_, String>(0))
        .expect("Failed to query")
        .filter_map(Result::ok)
        .collect()
}

/// `caller -> callee` pairs from both resolved call edges and unresolved call refs.
pub fn call_pairs(project_path: &Path) -> BTreeSet<String> {
    query_set(
        project_path,
        "SELECT s.name || ' -> ' || t.name FROM edges e
           JOIN nodes s ON s.id = e.source JOIN nodes t ON t.id = e.target
          WHERE e.kind = 'calls'
         UNION
         SELECT n.name || ' -> ' || u.reference_name FROM unresolved_refs u
           JOIN nodes n ON n.id = u.from_node_id
          WHERE u.reference_kind = 'calls'",
    )
}

/// `source -> target` names of stored edges of `kind` (`extends`, `calls`, …).
pub fn edges_of_kind(project_path: &Path, kind: &str) -> BTreeSet<String> {
    let conn = db::open_database(project_path).expect("Failed to open database");
    let mut stmt = conn
        .prepare(
            "SELECT s.name || ' -> ' || t.name FROM edges e
               JOIN nodes s ON s.id = e.source JOIN nodes t ON t.id = e.target
              WHERE e.kind = ?1",
        )
        .expect("Failed to prepare SQL statement");
    stmt.query_map([kind], |row| row.get::<_, String>(0))
        .expect("Failed to query")
        .filter_map(Result::ok)
        .collect()
}

/// `kind:name` for every node except files.
pub fn node_set(project_path: &Path) -> BTreeSet<String> {
    query_set(
        project_path,
        "SELECT kind || ':' || name FROM nodes WHERE kind != 'file'",
    )
}

/// `name | signature` of every import node.
pub fn import_set(project_path: &Path) -> BTreeSet<String> {
    query_set(
        project_path,
        "SELECT name || ' | ' || ifnull(signature, '') FROM nodes WHERE kind = 'import'",
    )
}

pub fn assert_contains_all(actual: &BTreeSet<String>, expected: &[&str]) {
    let missing: Vec<_> = expected.iter().filter(|e| !actual.contains(**e)).collect();
    assert!(missing.is_empty(), "missing {missing:?} in {actual:#?}");
}

pub fn assert_contains_none(actual: &BTreeSet<String>, unexpected: &[&str]) {
    let present: Vec<_> = unexpected.iter().filter(|e| actual.contains(**e)).collect();
    assert!(present.is_empty(), "unexpected {present:?} in {actual:#?}");
}

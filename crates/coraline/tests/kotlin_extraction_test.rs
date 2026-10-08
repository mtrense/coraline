//! Kotlin extraction fixture tests: calls, imports, declarations, exports.
#![allow(clippy::expect_used)]

use std::collections::BTreeSet;
use std::path::Path;

use coraline::{config, db, extraction};
use tempfile::TempDir;

/// Index `files` in a fresh project and return the project's temp dir.
fn index_project(files: &[(&str, &str)]) -> TempDir {
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

fn query_set(project_path: &Path, sql: &str) -> BTreeSet<String> {
    let conn = db::open_database(project_path).expect("Failed to open database");
    let mut stmt = conn.prepare(sql).expect("Failed to prepare SQL statement");
    stmt.query_map([], |row| row.get::<_, String>(0))
        .expect("Failed to query")
        .filter_map(Result::ok)
        .collect()
}

/// `caller -> callee` pairs from both resolved call edges and unresolved call refs.
fn call_pairs(project_path: &Path) -> BTreeSet<String> {
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

fn assert_contains_all(actual: &BTreeSet<String>, expected: &[&str]) {
    let missing: Vec<_> = expected.iter().filter(|e| !actual.contains(**e)).collect();
    assert!(missing.is_empty(), "missing {missing:?} in {actual:#?}");
}

#[test]
fn kotlin_calls_are_extracted() {
    let temp = index_project(&[(
        "src/Main.kt",
        "class Foo {\n\
         \x20   companion object {\n\
         \x20       fun make(): Foo = Foo()\n\
         \x20   }\n\
         \x20   fun build() {}\n\
         }\n\
         \n\
         fun helper() {}\n\
         \n\
         fun main() {\n\
         \x20   helper()\n\
         \x20   val svc = Foo.make()\n\
         \x20   svc.build()\n\
         \x20   a.b.c.deep(1)\n\
         \x20   run { inner() }\n\
         \x20   listOf(1).map { it }.filter { true }\n\
         \x20   obj?.safe()\n\
         \x20   generic<Int>()\n\
         }\n",
    )]);

    assert_contains_all(
        &call_pairs(temp.path()),
        &[
            "main -> helper",
            "main -> make",
            "main -> build",
            "main -> deep",
            "main -> run",
            "main -> inner",
            "main -> listOf",
            "main -> map",
            "main -> filter",
            "main -> safe",
            "main -> generic",
            "make -> Foo",
        ],
    );
}

/// `file -> import name` pairs from import edges.
fn import_pairs(project_path: &Path) -> BTreeSet<String> {
    query_set(
        project_path,
        "SELECT s.name || ' -> ' || t.name FROM edges e
           JOIN nodes s ON s.id = e.source JOIN nodes t ON t.id = e.target
          WHERE e.kind = 'imports' AND t.kind = 'import'",
    )
}

#[test]
fn kotlin_imports_are_extracted() {
    let temp = index_project(&[(
        "src/Main.kt",
        "package com.example.app\n\
         \n\
         import com.example.model.Circle\n\
         import com.example.model.Point\n\
         \n\
         fun main() {}\n",
    )]);

    assert_contains_all(
        &import_pairs(temp.path()),
        &["Main.kt -> Circle", "Main.kt -> Point"],
    );
}

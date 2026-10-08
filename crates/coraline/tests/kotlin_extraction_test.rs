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

/// `kind:name` for every node except files.
fn node_set(project_path: &Path) -> BTreeSet<String> {
    query_set(
        project_path,
        "SELECT kind || ':' || name FROM nodes WHERE kind != 'file'",
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
        ],
    );
    // `Foo()` calls the class: an instantiation.
    assert_contains_all(
        &query_set(
            temp.path(),
            "SELECT s.name || ' -> ' || t.name FROM edges e
               JOIN nodes s ON s.id = e.source JOIN nodes t ON t.id = e.target
              WHERE e.kind = 'instantiates'",
        ),
        &["make -> Foo"],
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

#[test]
fn kotlin_import_paths_handle_alias_and_wildcard() {
    let temp = index_project(&[(
        "src/Main.kt",
        "package com.example.app\n\
         \n\
         import com.example.model.Circle\n\
         import com.example.model.format as fmt\n\
         import com.example.util.*\n\
         \n\
         fun main() {}\n",
    )]);

    let imports = query_set(
        temp.path(),
        "SELECT name || ' | ' || signature FROM nodes WHERE kind = 'import'",
    );
    let expected: BTreeSet<String> = [
        "Circle | com.example.model.Circle|export=Circle",
        "fmt | com.example.model.format|export=format",
        "* | com.example.util",
    ]
    .iter()
    .map(|s| (*s).to_string())
    .collect();
    assert_eq!(imports, expected);
}

#[test]
fn kotlin_properties_are_extracted() {
    let temp = index_project(&[(
        "src/Main.kt",
        "private val secret = 1\n\
         var counter: Int = 0\n\
         \n\
         object Registry {\n\
         \x20   val shapes = mutableListOf<String>()\n\
         }\n",
    )]);

    assert_contains_all(
        &node_set(temp.path()),
        &["property:secret", "property:counter", "property:shapes"],
    );
}

#[test]
fn kotlin_interfaces_are_extracted() {
    let temp = index_project(&[(
        "src/Main.kt",
        "interface Named {\n\
         \x20   fun name(): String\n\
         }\n\
         \n\
         fun interface Action {\n\
         \x20   fun run()\n\
         }\n\
         \n\
         class Impl : Named {\n\
         \x20   override fun name(): String = \"impl\"\n\
         }\n",
    )]);

    let nodes = node_set(temp.path());
    assert_contains_all(
        &nodes,
        &["interface:Named", "interface:Action", "class:Impl"],
    );
    assert!(!nodes.contains("class:Named"), "{nodes:#?}");
}

#[test]
fn kotlin_enums_and_entries_are_extracted() {
    let temp = index_project(&[(
        "src/Main.kt",
        "enum class Color(val rgb: Int) {\n\
         \x20   RED(1), GREEN(2);\n\
         \x20   fun hex(): String = \"x\"\n\
         }\n\
         \n\
         enum class Plain { A, B }\n",
    )]);

    let nodes = node_set(temp.path());
    assert_contains_all(
        &nodes,
        &[
            "enum:Color",
            "enum_member:RED",
            "enum_member:GREEN",
            "function:hex",
            "enum:Plain",
            "enum_member:A",
            "enum_member:B",
        ],
    );
    assert!(!nodes.contains("class:Color"), "{nodes:#?}");
}

#[test]
fn kotlin_package_constructors_companions_and_type_aliases_are_extracted() {
    let temp = index_project(&[(
        "src/Main.kt",
        "package com.example.app\n\
         \n\
         typealias Names = List<String>\n\
         \n\
         class Foo(val x: Int) {\n\
         \x20   constructor(s: String) : this(s.length) {\n\
         \x20       bar()\n\
         \x20   }\n\
         \x20   companion object {\n\
         \x20       fun make(): Foo = Foo(1)\n\
         \x20   }\n\
         \x20   fun bar() {}\n\
         }\n\
         \n\
         class Baz {\n\
         \x20   companion object Factory {\n\
         \x20       fun create(): Baz = Baz()\n\
         \x20   }\n\
         }\n",
    )]);

    assert_contains_all(
        &node_set(temp.path()),
        &[
            "module:com.example.app",
            "type_alias:Names",
            "method:constructor",
            "class:Companion",
            "class:Factory",
            "function:make",
            "function:create",
        ],
    );
    assert_contains_all(&call_pairs(temp.path()), &["constructor -> bar"]);
    assert_contains_all(
        &query_set(
            temp.path(),
            "SELECT qualified_name FROM nodes WHERE name IN ('make', 'create')",
        ),
        &[
            "src/Main.kt::Foo::Companion::make",
            "src/Main.kt::Baz::Factory::create",
        ],
    );
}

#[test]
fn kotlin_top_level_declarations_are_implicitly_exported() {
    let temp = index_project(&[
        (
            "src/model/Shape.kt",
            "package com.example.model\n\
             \n\
             interface Named\n\
             class Circle {\n\
             \x20   fun member() {}\n\
             }\n\
             object Registry\n\
             fun format(d: Double): String = d.toString()\n\
             internal fun internalHelper() {}\n\
             private fun hidden() {}\n\
             val shared = 1\n\
             private val secret = 2\n\
             typealias Names = List<String>\n",
        ),
        ("src/Script.kt", "fun topLevel() {}\n"),
    ]);

    let exports = query_set(
        temp.path(),
        "SELECT t.name || ' | ' || t.signature FROM edges e
           JOIN nodes s ON s.id = e.source JOIN nodes t ON t.id = e.target
          WHERE e.kind = 'exports' AND s.kind = 'file'",
    );
    let expected: BTreeSet<String> = [
        "Named | com.example.model.Named",
        "Circle | com.example.model.Circle",
        "Registry | com.example.model.Registry",
        "format | com.example.model.format",
        "internalHelper | com.example.model.internalHelper",
        "shared | com.example.model.shared",
        "Names | com.example.model.Names",
        "topLevel | topLevel",
    ]
    .iter()
    .map(|s| (*s).to_string())
    .collect();
    assert_eq!(exports, expected);
}

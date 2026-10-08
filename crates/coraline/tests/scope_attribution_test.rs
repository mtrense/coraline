//! Calls outside any function or method (property initializers, Kotlin
//! `init {}` blocks and getters, class-field arrow functions, top-level
//! script code) are attributed to the enclosing type, else to the file.
#![allow(clippy::expect_used)]

mod common;

use std::collections::BTreeSet;
use std::path::Path;

use common::{assert_contains_all, assert_contains_none, call_pairs, index_project, query_set};

/// `caller -> callee` for every stored calls edge.
fn call_edges(project_path: &Path) -> BTreeSet<String> {
    query_set(
        project_path,
        "SELECT s.name || ' -> ' || t.name FROM edges e
           JOIN nodes s ON s.id = e.source JOIN nodes t ON t.id = e.target
          WHERE e.kind = 'calls'",
    )
}

#[test]
fn kotlin_initializers_init_blocks_and_getters() {
    let temp = index_project(&[(
        "src/Box.kt",
        "package app\n\
         \n\
         fun helper(): Int = 1\n\
         fun other(): Int = 2\n\
         \n\
         class Box {\n\
         \x20   val size = helper()\n\
         \x20   val area: Int\n\
         \x20       get() = other()\n\
         \n\
         \x20   init {\n\
         \x20       setup()\n\
         \x20   }\n\
         \n\
         \x20   fun setup() {\n\
         \x20       println(\"x\")\n\
         \x20   }\n\
         }\n\
         \n\
         val top = helper()\n",
    )]);
    let edges = call_edges(temp.path());
    assert_contains_all(
        &edges,
        &[
            "Box -> helper",
            "Box -> other",
            "Box -> setup",
            "Box.kt -> helper",
        ],
    );
    // Calls inside methods stay attributed to the method.
    let calls = call_pairs(temp.path());
    assert_contains_all(&calls, &["setup -> println"]);
    assert_contains_none(&calls, &["Box -> println", "Box.kt -> println"]);
}

#[test]
fn typescript_class_fields_and_top_level_code() {
    let temp = index_project(&[(
        "src/w.ts",
        "function helper() { return 1; }\n\
         function other() { return 2; }\n\
         function main() {}\n\
         \n\
         class Widget {\n\
         \x20 size = helper();\n\
         \x20 onClick = () => other();\n\
         }\n\
         \n\
         const handlers = { run: () => helper() };\n\
         main();\n",
    )]);
    assert_contains_all(
        &call_edges(temp.path()),
        &[
            "Widget -> helper",
            "Widget -> other",
            "w.ts -> helper",
            "w.ts -> main",
        ],
    );
}

#[test]
fn python_class_body_and_script_code() {
    let temp = index_project(&[(
        "app/main.py",
        "def helper():\n\
         \x20   return 1\n\
         \n\
         class Config:\n\
         \x20   value = helper()\n\
         \n\
         if __name__ == \"__main__\":\n\
         \x20   helper()\n",
    )]);
    assert_contains_all(
        &call_edges(temp.path()),
        &["Config -> helper", "main.py -> helper"],
    );
}

#[test]
fn java_field_initializers() {
    let temp = index_project(&[(
        "src/Service.java",
        "class Service {\n\
         \x20   static int count = compute();\n\
         \n\
         \x20   static int compute() {\n\
         \x20       return 1;\n\
         \x20   }\n\
         }\n",
    )]);
    assert_contains_all(&call_edges(temp.path()), &["Service -> compute"]);
}

#[test]
fn go_package_level_vars() {
    let temp = index_project(&[(
        "pkg/a.go",
        "package pkg\n\
         \n\
         var defaultName = build()\n\
         \n\
         func build() string { return \"\" }\n",
    )]);
    assert_contains_all(&call_edges(temp.path()), &["a.go -> build"]);
}

/// An unqualified call in an initializer means the type's own member.
#[test]
fn initializer_calls_prefer_own_members() {
    let temp = index_project(&[(
        "src/Two.kt",
        "package app\n\
         \n\
         class A {\n\
         \x20   init {\n\
         \x20       setup()\n\
         \x20   }\n\
         \x20   fun setup() {}\n\
         }\n\
         \n\
         class B {\n\
         \x20   fun setup() {}\n\
         }\n",
    )]);
    let edges = query_set(
        temp.path(),
        "SELECT s.name || ' -> ' || p.name || '.' || t.name FROM edges e
           JOIN nodes s ON s.id = e.source JOIN nodes t ON t.id = e.target
           JOIN edges c ON c.target = t.id AND c.kind = 'contains'
           JOIN nodes p ON p.id = c.source
          WHERE e.kind = 'calls'",
    );
    assert_eq!(edges, BTreeSet::from(["A -> A.setup".to_string()]));
}

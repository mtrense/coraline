//! Swift extraction fixture tests.

mod common;

use common::{
    assert_contains_all, assert_contains_none, call_pairs, import_set, index_project, node_set,
    query_set,
};

#[test]
fn swift_calls_are_extracted() {
    let temp = index_project(&[(
        "Sources/App/Main.swift",
        "class Service {\n\
         \x20   init() { setup() }\n\
         \x20   func setup() {}\n\
         \x20   func build() {\n\
         \x20       helper()\n\
         \x20       self.run()\n\
         \x20       a.b.deep(1)\n\
         \x20       let x = Point(x: 1)\n\
         \x20       obj?.safe()\n\
         \x20       list.map { $0 }.filter { true }\n\
         \x20       Box<Int>()\n\
         \x20   }\n\
         }\n",
    )]);

    assert_contains_all(
        &call_pairs(temp.path()),
        &[
            "init -> setup",
            "build -> helper",
            "build -> run",
            "build -> deep",
            "build -> Point",
            "build -> safe",
            "build -> map",
            "build -> filter",
            "build -> Box",
        ],
    );
}

#[test]
fn swift_declarations_are_extracted() {
    let temp = index_project(&[(
        "Sources/App/Shapes.swift",
        "class Base {\n\
         \x20   deinit { cleanup() }\n\
         }\n\
         struct Point { func len() {} }\n\
         enum Color { case red\n\
         \x20   func hex() {} }\n\
         extension Point { func scaled() {} }\n\
         actor Counter {}\n\
         protocol Shape { func area() }\n",
    )]);

    let nodes = node_set(temp.path());
    assert_contains_all(
        &nodes,
        &[
            "class:Base",
            "struct:Point",
            "enum:Color",
            "class:Counter",
            "protocol:Shape",
            "method:deinit",
            "function:len",
            "function:hex",
            "function:scaled",
        ],
    );
    assert_contains_none(&nodes, &["class:Color"]);
    assert_contains_all(&call_pairs(temp.path()), &["deinit -> cleanup"]);
    assert_contains_all(
        &query_set(
            temp.path(),
            "SELECT qualified_name FROM nodes WHERE name = 'scaled'",
        ),
        &["Sources/App/Shapes.swift::Point::scaled"],
    );
}

#[test]
fn swift_imports_are_extracted() {
    let temp = index_project(&[(
        "Sources/App/Main.swift",
        "import Foundation\n\
         import struct Models.Point\n\
         \n\
         func main() {}\n",
    )]);

    assert_contains_all(
        &import_set(temp.path()),
        &["Foundation | Foundation", "Models.Point | Models.Point"],
    );
}

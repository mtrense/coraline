//! Swift extraction fixture tests.

mod common;

use common::{assert_contains_all, call_pairs, index_project};

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

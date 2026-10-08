//! Rust extraction fixture tests.

mod common;

use common::{assert_contains_all, call_pairs, import_set, index_project, query_set};

#[test]
fn rust_imports_are_extracted() {
    let temp = index_project(&[(
        "src/lib.rs",
        "use std::fmt;\n\
         use crate::model::Circle;\n\
         use model::{format_num, Point as P, shapes::{Square, self}};\n\
         use std::io::{self, Write};\n\
         use std::collections::*;\n\
         use serde as sd;\n\
         use ::alloc::vec::Vec;\n\
         pub fn f() {}\n",
    )]);

    assert_contains_all(
        &import_set(temp.path()),
        &[
            "fmt | std::fmt",
            "Circle | crate::model::Circle",
            "format_num | model::format_num",
            "P | model::Point|export=Point",
            "Square | model::shapes::Square",
            "shapes | model::shapes",
            "io | std::io",
            "Write | std::io::Write",
            "* | std::collections::*",
            "sd | serde|export=serde",
            "Vec | ::alloc::vec::Vec",
        ],
    );
}

#[test]
fn rust_pub_use_is_reexported() {
    let temp = index_project(&[(
        "src/lib.rs",
        "mod model;\n\
         pub use model::Circle;\n\
         pub use model::{Point, Square as Sq};\n\
         pub(crate) use model::Hidden;\n\
         use model::Private;\n",
    )]);

    let exports = query_set(
        temp.path(),
        "SELECT name || ' | ' || ifnull(signature, '') FROM nodes WHERE kind = 'export'",
    );
    assert_eq!(
        exports.into_iter().collect::<Vec<_>>(),
        [
            "Circle | model::Circle",
            "Point | model::Point",
            "Sq | model::Square",
        ],
    );
    // A `pub use` still imports the names into the module.
    assert_contains_all(
        &import_set(temp.path()),
        &["Circle | model::Circle", "Private | model::Private"],
    );
}

#[test]
fn rust_macro_invocations_are_calls() {
    let temp = index_project(&[(
        "src/lib.rs",
        "fn run() {\n\
         \x20   helper();\n\
         \x20   println!(\"x\");\n\
         \x20   std::assert_eq!(1, 1);\n\
         }\n\
         fn helper() {}\n",
    )]);

    assert_contains_all(
        &call_pairs(temp.path()),
        &["run -> helper", "run -> println", "run -> assert_eq"],
    );
}

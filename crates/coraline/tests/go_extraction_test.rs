//! Go extraction fixture tests.

mod common;

use common::{assert_contains_all, import_set, index_project};

#[test]
fn go_imports_are_extracted() {
    let temp = index_project(&[
        (
            "model/model.go",
            "package model\n\nimport \"fmt\"\n\nfunc F() { fmt.Println() }\n",
        ),
        (
            "service/service.go",
            "package service\n\n\
             import (\n\
             \t\"strings\"\n\
             \n\
             \tm \"example.com/app/model\"\n\
             \t_ \"embed\"\n\
             \t. \"example.com/app/dot\"\n\
             )\n\n\
             func G() { m.F() }\n",
        ),
    ]);

    assert_contains_all(
        &import_set(temp.path()),
        &[
            "fmt | fmt",
            "strings | strings",
            "m | example.com/app/model",
            "_ | embed",
            ". | example.com/app/dot",
        ],
    );
}

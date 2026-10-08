//! Go extraction fixture tests.

mod common;

use common::{assert_contains_all, call_pairs, import_set, index_project};

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

#[test]
fn go_call_names_are_identifiers() {
    let temp = index_project(&[(
        "main.go",
        "package main\n\n\
         func run(xs []int) {\n\
         \thelper(1)\n\
         \tmodel.NewCircle(2.0)\n\
         \ta.b.Deep()\n\
         \tMap[int](xs, 1)\n\
         \tpkg.Gen[string]()\n\
         \tfunc() { inner(1) }()\n\
         \tfor _, i := range xs {\n\
         \t\tfunc() { helper2(i) }()\n\
         \t}\n\
         \tgetFn()()\n\
         \t(handler)(1)\n\
         }\n",
    )]);

    let calls = call_pairs(temp.path());
    assert_contains_all(
        &calls,
        &[
            "run -> helper",
            "run -> NewCircle",
            "run -> Deep",
            "run -> Map",
            "run -> Gen",
            "run -> inner",
            "run -> helper2",
            "run -> getFn",
            "run -> handler",
        ],
    );
    // Callees of func literals / call results must not leak source text.
    let garbage: Vec<_> = calls
        .iter()
        .filter_map(|c| c.split_once(" -> ").map(|(_, callee)| callee))
        .filter(|callee| !callee.chars().all(|ch| ch.is_alphanumeric() || ch == '_'))
        .collect();
    assert!(garbage.is_empty(), "garbage callee names: {garbage:?}");
}

//! Java extraction fixture tests.

mod common;

use common::{assert_contains_all, call_pairs, import_set, index_project, node_set};

#[test]
fn java_calls_are_extracted() {
    let temp = index_project(&[(
        "src/Foo.java",
        "class Foo {\n\
         \x20   Foo() { init(); }\n\
         \x20   void init() {}\n\
         \x20   void build() {\n\
         \x20       helper();\n\
         \x20       this.run();\n\
         \x20       a.b.deep(1);\n\
         \x20       Util.<String>gen();\n\
         \x20       Foo f = new Foo();\n\
         \x20       new java.util.ArrayList<String>();\n\
         \x20       new Bar<Map<String, Integer>>();\n\
         \x20   }\n\
         }\n",
    )]);

    assert_contains_all(
        &call_pairs(temp.path()),
        &[
            "Foo -> init",
            "build -> helper",
            "build -> run",
            "build -> deep",
            "build -> gen",
            "build -> Foo",
            "build -> ArrayList",
            "build -> Bar",
        ],
    );
}

#[test]
fn java_package_and_wildcard_imports_are_extracted() {
    let temp = index_project(&[(
        "src/app/b/App.java",
        "package app.b;\n\
         \n\
         import app.a.*;\n\
         import static app.a.Util.*;\n\
         import app.a.Report;\n\
         \n\
         class App {}\n",
    )]);

    assert_contains_all(&node_set(temp.path()), &["module:app.b"]);
    let imports = import_set(temp.path());
    assert_contains_all(
        &imports,
        &[
            "* | app.a",
            "* | app.a.Util",
            "Report | app.a.Report|export=Report",
        ],
    );
    assert_eq!(imports.len(), 3, "{imports:#?}");
}

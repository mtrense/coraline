//! Java extraction fixture tests.

mod common;

use common::{assert_contains_all, call_pairs, index_project};

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

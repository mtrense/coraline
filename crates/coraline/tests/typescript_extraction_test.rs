//! TypeScript / JavaScript extraction fixture tests.

mod common;

use common::{assert_contains_all, call_pairs, index_project, node_set};

const SOURCE: &str = "export const f = () => { helper(); };\n\
                      const g = function () { other(); };\n\
                      const h = async (x) => x.run();\n\
                      function outer() {\n\
                      \x20 const inner = () => deep();\n\
                      \x20 [1].map((x) => mapped(x));\n\
                      }\n\
                      function helper() {}\n";

#[test]
fn arrow_functions_assigned_to_variables_are_named_scopes() {
    for file in ["src/a.ts", "src/a.js"] {
        let temp = index_project(&[(file, SOURCE)]);

        assert_contains_all(
            &node_set(temp.path()),
            &[
                "function:f",
                "function:g",
                "function:h",
                "function:inner",
                "function:outer",
            ],
        );
        assert_contains_all(
            &call_pairs(temp.path()),
            &[
                "f -> helper",
                "g -> other",
                "h -> run",
                "inner -> deep",
                // Anonymous callbacks stay attributed to the enclosing function.
                "outer -> map",
                "outer -> mapped",
            ],
        );
    }
}

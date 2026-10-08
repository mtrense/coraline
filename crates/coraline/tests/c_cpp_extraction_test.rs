//! C and C++ extraction fixture tests.

mod common;

use common::{assert_contains_all, assert_contains_none, call_pairs, index_project, node_set};

#[test]
fn c_functions_calls_and_macros_are_extracted() {
    let temp = index_project(&[(
        "src/main.c",
        "#include <stdio.h>\n\
         #define MAX 10\n\
         #define SQ(x) ((x) * (x))\n\
         \n\
         static int *make(int a) { helper(a); return 0; }\n\
         int (*getfn(void))(int) { use_it(); return 0; }\n\
         \n\
         int main(void) {\n\
         \x20   make(1);\n\
         \x20   obj.run();\n\
         \x20   p->go();\n\
         \x20   return SQ(2);\n\
         }\n",
    )]);

    assert_contains_all(
        &node_set(temp.path()),
        &[
            "function:make",
            "function:getfn",
            "function:main",
            "constant:MAX",
            "function:SQ",
        ],
    );
    assert_contains_all(
        &call_pairs(temp.path()),
        &[
            "make -> helper",
            "getfn -> use_it",
            "main -> make",
            "main -> run",
            "main -> go",
            "main -> SQ",
        ],
    );
}

#[test]
fn cpp_methods_namespaces_and_calls_are_extracted() {
    let temp = index_project(&[(
        "src/shape.cpp",
        "#include <vector>\n\
         #define MAX 10\n\
         namespace geo {\n\
         class Shape {\n\
         public:\n\
         \x20   Shape() { init(); }\n\
         \x20   ~Shape() {}\n\
         \x20   double area() const { return calc<int>(1); }\n\
         \x20   void decl();\n\
         };\n\
         \n\
         void Shape::decl() {\n\
         \x20   std::sort(a, b);\n\
         \x20   obj.run();\n\
         \x20   p->go();\n\
         \x20   this->area();\n\
         }\n\
         \n\
         template <typename T> T tmax(T a) { return pick(a); }\n\
         }\n",
    )]);

    let nodes = node_set(temp.path());
    assert_contains_all(
        &nodes,
        &[
            "namespace:geo",
            "class:Shape",
            "method:Shape",
            "method:~Shape",
            "method:area",
            "method:decl",
            "function:tmax",
            "constant:MAX",
        ],
    );
    assert_contains_none(&nodes, &["function:decl", "function:area"]);
    assert_contains_all(
        &call_pairs(temp.path()),
        &[
            "Shape -> init",
            "area -> calc",
            "decl -> sort",
            "decl -> run",
            "decl -> go",
            "decl -> area",
            "tmax -> pick",
        ],
    );
}

//! `Instantiates` edges: explicit constructions (`new Foo()`, `Foo.new`,
//! `Foo{}`, `Foo<T>()`) and calls whose callee resolves to a class / struct
//! (`Circle(2.0)` in Kotlin, Python, Swift, C++). Resolved like calls (same
//! file, same dir, imports, packages / namespaces / modules; never by name
//! alone, upstream #43). The edge points at the type, never at a
//! constructor method.
#![allow(clippy::expect_used)]

mod common;

use common::{assert_contains_all, assert_contains_none, edges_of_kind, index_project};

fn assert_instantiates(files: &[(&str, &str)], expected: &[&str]) {
    let temp = index_project(files);
    let instantiates = edges_of_kind(temp.path(), "instantiates");
    assert_contains_all(&instantiates, expected);
    // Constructions are not calls (neither to the type nor to a
    // constructor named like it).
    assert_contains_none(&edges_of_kind(temp.path(), "calls"), expected);
}

#[test]
fn kotlin_constructor_calls_through_imports() {
    assert_instantiates(
        &[
            (
                "a/Shapes.kt",
                "package app.a\n\
                 \n\
                 class Circle(val r: Double)\n\
                 \n\
                 class Square(val s: Double) {\n\
                 \x20   fun twice() = Square(s * 2)\n\
                 }\n",
            ),
            (
                "b/App.kt",
                "package app.b\n\
                 \n\
                 import app.a.Circle\n\
                 \n\
                 fun run() {\n\
                 \x20   Circle(1.0)\n\
                 }\n",
            ),
        ],
        &["run -> Circle", "twice -> Square"],
    );
}

#[test]
fn python_constructor_calls() {
    assert_instantiates(
        &[
            (
                "a/shapes.py",
                "class Circle:\n\
                 \x20   def __init__(self, r):\n\
                 \x20       self.r = r\n",
            ),
            (
                "b/app.py",
                "from a.shapes import Circle\n\
                 \n\
                 \n\
                 def run():\n\
                 \x20   return Circle(1.0)\n",
            ),
        ],
        &["run -> Circle"],
    );
}

#[test]
fn swift_initializer_calls_and_constructor_expressions() {
    assert_instantiates(
        &[
            ("App.xcodeproj/project.pbxproj", "// !$*UTF8*$!\n"),
            (
                "a/Shapes.swift",
                "class Circle {\n\
                 \x20   init(r: Double) {}\n\
                 }\n\
                 \n\
                 struct Box<T> {}\n",
            ),
            (
                "b/App.swift",
                "func run() {\n\
                 \x20   _ = Circle(r: 1.0)\n\
                 }\n\
                 \n\
                 func pack() {\n\
                 \x20   _ = Box<Int>()\n\
                 }\n",
            ),
        ],
        &["run -> Circle", "pack -> Box"],
    );
}

/// `new Circle()` links the class, not the constructor method `Circle`.
#[test]
fn java_new_links_the_class() {
    assert_instantiates(
        &[
            (
                "a/Circle.java",
                "package app.a;\n\
                 \n\
                 public class Circle {\n\
                 \x20   public Circle(double r) {}\n\
                 }\n",
            ),
            (
                "b/App.java",
                "package app.b;\n\
                 \n\
                 import app.a.Circle;\n\
                 \n\
                 class App {\n\
                 \x20   static void run() { new Circle(1.0); }\n\
                 \x20   static void full() { new app.a.Circle(2.0); }\n\
                 }\n",
            ),
            (
                "c/Other.java",
                "package app.c;\n\
                 \n\
                 class Other {\n\
                 \x20   static void qualified() { new app.a.Circle(3.0); }\n\
                 }\n",
            ),
        ],
        &["run -> Circle", "full -> Circle", "qualified -> Circle"],
    );
}

#[test]
fn csharp_new_through_using() {
    assert_instantiates(
        &[
            (
                "a/Circle.cs",
                "namespace App.A\n\
                 {\n\
                 \x20   public class Circle\n\
                 \x20   {\n\
                 \x20       public Circle(double r) {}\n\
                 \x20   }\n\
                 }\n",
            ),
            (
                "b/App.cs",
                "using App.A;\n\
                 \n\
                 namespace App.B\n\
                 {\n\
                 \x20   static class Program\n\
                 \x20   {\n\
                 \x20       static void Run() { var c = new Circle(1.0); }\n\
                 \x20   }\n\
                 }\n",
            ),
        ],
        &["Run -> Circle"],
    );
}

#[test]
fn typescript_new_expression() {
    assert_instantiates(
        &[
            (
                "a/shapes.ts",
                "export class Circle {\n  constructor(private r: number) {}\n}\n",
            ),
            (
                "b/app.ts",
                "import { Circle } from '../a/shapes';\n\
                 \n\
                 export function run(): Circle {\n\
                 \x20 return new Circle(1);\n\
                 }\n",
            ),
        ],
        &["run -> Circle"],
    );
}

#[test]
fn php_new_through_use() {
    assert_instantiates(
        &[
            (
                "a/Circle.php",
                "<?php\n\
                 namespace App\\A;\n\
                 \n\
                 class Circle {\n\
                 \x20   public function __construct(float $r) {}\n\
                 }\n",
            ),
            (
                "b/App.php",
                "<?php\n\
                 namespace App\\B;\n\
                 \n\
                 use App\\A\\Circle;\n\
                 \n\
                 function run(): void { new Circle(1.0); }\n",
            ),
        ],
        &["run -> Circle"],
    );
}

#[test]
fn ruby_new_through_require() {
    assert_instantiates(
        &[
            (
                "a/shapes.rb",
                "module Geo\n\
                 \x20 class Ring\n\
                 \x20 end\n\
                 end\n\
                 \n\
                 class Circle\n\
                 \x20 def initialize(r)\n\
                 \x20   @r = r\n\
                 \x20 end\n\
                 end\n",
            ),
            (
                "b/app.rb",
                "require_relative '../a/shapes'\n\
                 \n\
                 def run\n\
                 \x20 Circle.new(1.0)\n\
                 \x20 Geo::Ring.new\n\
                 end\n",
            ),
        ],
        &["run -> Circle", "run -> Ring"],
    );
}

#[test]
fn cpp_new_and_temporaries() {
    assert_instantiates(
        &[
            (
                "a/shapes.hpp",
                "class Circle {\n\
                 public:\n\
                 \x20   Circle(double r) {}\n\
                 };\n",
            ),
            (
                "b/app.cpp",
                "#include \"../a/shapes.hpp\"\n\
                 \n\
                 void run() {\n\
                 \x20   Circle *c = new Circle(1.0);\n\
                 }\n\
                 \n\
                 void temp() {\n\
                 \x20   Circle(2.0);\n\
                 }\n",
            ),
        ],
        &["run -> Circle", "temp -> Circle"],
    );
}

#[test]
fn go_composite_literals() {
    assert_instantiates(
        &[
            ("go.mod", "module example.com/app\n"),
            (
                "a/shapes.go",
                "package a\n\
                 \n\
                 type Circle struct {\n\
                 \x20   R float64\n\
                 }\n\
                 \n\
                 func Unit() *Circle { return &Circle{R: 1} }\n",
            ),
            (
                "b/app.go",
                "package main\n\
                 \n\
                 import \"example.com/app/a\"\n\
                 \n\
                 func Run() {\n\
                 \x20   _ = a.Circle{R: 1.0}\n\
                 \x20   _ = []int{1}\n\
                 }\n",
            ),
        ],
        &["Run -> Circle", "Unit -> Circle"],
    );
}

#[test]
fn rust_struct_expressions() {
    assert_instantiates(
        &[
            ("src/shapes.rs", "pub struct Circle {\n    pub r: f64,\n}\n"),
            (
                "src/app.rs",
                "use crate::shapes::Circle;\n\
                 \n\
                 fn run() -> Circle {\n\
                 \x20   Circle { r: 1.0 }\n\
                 }\n",
            ),
        ],
        &["run -> Circle"],
    );
}

/// Like calls (#43): a class in an unrelated directory with no import /
/// package link is not linked by name alone.
#[test]
fn no_import_no_instantiates_edge() {
    for files in [
        &[
            ("x/Shapes.kt", "package x\n\nclass Circle\n"),
            ("y/App.kt", "package y\n\nfun run() { Circle() }\n"),
        ][..],
        &[
            ("x/shapes.py", "class Circle:\n    pass\n"),
            ("y/app.py", "def run():\n    return Circle()\n"),
        ][..],
        &[
            ("x/shapes.ts", "export class Circle {}\n"),
            ("y/app.ts", "function run() { return new Circle(); }\n"),
        ][..],
    ] {
        let temp = index_project(files);
        let instantiates = edges_of_kind(temp.path(), "instantiates");
        assert!(instantiates.is_empty(), "unexpected {instantiates:?}");
        let calls = edges_of_kind(temp.path(), "calls");
        assert!(calls.is_empty(), "unexpected {calls:?}");
    }
}

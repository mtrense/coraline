//! `Extends` / `Implements` edges: supertypes named in class / interface
//! declarations, resolved like calls (same file, same dir, imports,
//! packages / namespaces / modules; never by name alone, upstream #43).
//!
//! Where the syntax doesn't tell (Kotlin `: I`, C# `: I`, Swift `: P`,
//! Python `(Base)`), a class / struct / object naming an interface /
//! protocol / trait implements it; everything else extends.
#![allow(clippy::expect_used)]

mod common;

use common::{assert_contains_all, assert_contains_none, edges_of_kind, index_project};

fn assert_inheritance(files: &[(&str, &str)], extends: &[&str], implements: &[&str]) {
    let temp = index_project(files);
    let actual_extends = edges_of_kind(temp.path(), "extends");
    let actual_implements = edges_of_kind(temp.path(), "implements");
    assert_contains_all(&actual_extends, extends);
    assert_contains_all(&actual_implements, implements);
    // Each supertype gets exactly one of the two kinds.
    assert_contains_none(&actual_extends, implements);
    assert_contains_none(&actual_implements, extends);
}

#[test]
fn kotlin_delegation_specifiers() {
    assert_inheritance(
        &[
            (
                "a/Base.kt",
                "package app.a\n\
                 \n\
                 interface Named\n\
                 \n\
                 interface Drawable : Named {\n\
                 \x20   fun draw()\n\
                 }\n\
                 \n\
                 abstract class Shape(val n: Int)\n",
            ),
            (
                "b/Circle.kt",
                "package app.b\n\
                 \n\
                 import app.a.Drawable\n\
                 import app.a.Shape\n\
                 \n\
                 class Circle : Shape(1), Drawable {\n\
                 \x20   override fun draw() {}\n\
                 }\n\
                 \n\
                 object Origin : Drawable {\n\
                 \x20   override fun draw() {}\n\
                 }\n",
            ),
        ],
        &["Circle -> Shape", "Drawable -> Named"],
        &["Circle -> Drawable", "Origin -> Drawable"],
    );
}

#[test]
fn java_superclass_and_interfaces() {
    assert_inheritance(
        &[
            (
                "a/Shape.java",
                "package app.a;\n\npublic abstract class Shape {}\n",
            ),
            (
                "a/Named.java",
                "package app.a;\n\npublic interface Named {}\n",
            ),
            (
                "a/Drawable.java",
                "package app.a;\n\npublic interface Drawable extends Named {}\n",
            ),
            (
                "b/Circle.java",
                "package app.b;\n\
                 \n\
                 import app.a.*;\n\
                 \n\
                 public class Circle extends Shape implements Drawable, java.io.Serializable {}\n\
                 \n\
                 enum Kind implements Drawable { ROUND }\n",
            ),
        ],
        &["Circle -> Shape", "Drawable -> Named"],
        &["Circle -> Drawable", "Kind -> Drawable"],
    );
}

#[test]
fn csharp_base_list() {
    assert_inheritance(
        &[
            (
                "a/Shapes.cs",
                "namespace App.A\n\
                 {\n\
                 \x20   public abstract class Shape {}\n\
                 \x20   public interface IDrawable {}\n\
                 \x20   public interface IShape : IDrawable {}\n\
                 }\n",
            ),
            (
                "b/Circle.cs",
                "using App.A;\n\
                 \n\
                 namespace App.B\n\
                 {\n\
                 \x20   public class Circle : Shape, IShape {}\n\
                 \x20   public struct Point : IDrawable {}\n\
                 }\n",
            ),
        ],
        &["Circle -> Shape", "IShape -> IDrawable"],
        &["Circle -> IShape", "Point -> IDrawable"],
    );
}

#[test]
fn typescript_class_heritage() {
    assert_inheritance(
        &[
            (
                "a/shapes.ts",
                "export interface Named {}\n\
                 export interface Drawable extends Named {}\n\
                 export abstract class Shape {}\n",
            ),
            (
                "b/circle.ts",
                "import { Drawable, Shape } from '../a/shapes';\n\
                 \n\
                 export class Circle extends Shape implements Drawable {}\n",
            ),
        ],
        &["Circle -> Shape", "Drawable -> Named"],
        &["Circle -> Drawable"],
    );
}

#[test]
fn swift_inheritance_specifiers() {
    assert_inheritance(
        &[
            ("App.xcodeproj/project.pbxproj", "// !$*UTF8*$!\n"),
            (
                "a/Shapes.swift",
                "protocol Named {}\n\
                 protocol Drawable: Named {}\n\
                 class Shape {}\n",
            ),
            (
                "b/Circle.swift",
                "class Circle: Shape, Drawable {}\n\
                 struct Point: Drawable {}\n",
            ),
        ],
        &["Circle -> Shape", "Drawable -> Named"],
        &["Circle -> Drawable", "Point -> Drawable"],
    );
}

#[test]
fn python_superclasses() {
    assert_inheritance(
        &[
            (
                "a/shapes.py",
                "class Shape:\n    pass\n\n\nclass Meta(type):\n    pass\n",
            ),
            (
                "b/circle.py",
                "from a.shapes import Meta, Shape\n\
                 \n\
                 \n\
                 class Circle(Shape, metaclass=Meta):\n\
                 \x20   pass\n",
            ),
        ],
        &["Circle -> Shape"],
        &[],
    );
}

#[test]
fn ruby_superclass() {
    assert_inheritance(
        &[
            (
                "a/shapes.rb",
                "module Geo\n\
                 \x20 class Base\n\
                 \x20 end\n\
                 end\n\
                 \n\
                 class Shape\n\
                 end\n",
            ),
            (
                "b/circle.rb",
                "require_relative '../a/shapes'\n\
                 \n\
                 class Circle < Shape\n\
                 end\n\
                 \n\
                 class Ring < Geo::Base\n\
                 end\n",
            ),
        ],
        &["Circle -> Shape", "Ring -> Base"],
        &[],
    );
}

#[test]
fn php_base_and_interface_clauses() {
    assert_inheritance(
        &[
            (
                "a/Shapes.php",
                "<?php\n\
                 namespace App\\A;\n\
                 \n\
                 interface Named {}\n\
                 interface Drawable extends Named {}\n\
                 abstract class Shape {}\n",
            ),
            (
                "b/Circle.php",
                "<?php\n\
                 namespace App\\B;\n\
                 \n\
                 use App\\A\\Shape;\n\
                 use App\\A\\Drawable;\n\
                 \n\
                 class Circle extends Shape implements Drawable {}\n",
            ),
        ],
        &["Circle -> Shape", "Drawable -> Named"],
        &["Circle -> Drawable"],
    );
}

#[test]
fn cpp_base_class_clause() {
    assert_inheritance(
        &[
            (
                "a/shapes.hpp",
                "class Shape {};\n\
                 namespace geo { class Named {}; }\n",
            ),
            (
                "b/circle.cpp",
                "#include \"../a/shapes.hpp\"\n\
                 \n\
                 class Circle : public Shape, private geo::Named {};\n\
                 struct Point : Shape {};\n",
            ),
        ],
        &["Circle -> Shape", "Circle -> Named", "Point -> Shape"],
        &[],
    );
}

/// Like calls (#43): a supertype in an unrelated directory with no import /
/// package link is not linked by name alone.
#[test]
fn no_import_no_inheritance_edge() {
    for files in [
        &[
            ("x/Base.kt", "package x\n\nopen class Base\n"),
            ("y/Child.kt", "package y\n\nclass Child : Base()\n"),
        ][..],
        &[
            ("x/base.py", "class Base:\n    pass\n"),
            ("y/child.py", "class Child(Base):\n    pass\n"),
        ][..],
        &[
            ("x/base.ts", "export class Base {}\n"),
            ("y/child.ts", "class Child extends Base {}\n"),
        ][..],
    ] {
        let temp = index_project(files);
        let extends = edges_of_kind(temp.path(), "extends");
        assert!(extends.is_empty(), "unexpected {extends:?}");
    }
}

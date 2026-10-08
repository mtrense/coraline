//! Per-language edge fixture tests: index a small multi-directory project per
//! language through the real pipeline and assert the stored `Calls` and
//! `Imports` edges plus `coraline_callers` / `coraline_callees` retrieval.
//!
//! Every fixture has the same shape:
//!
//! - `a/shapes`: base type `Shape`, subtype `Circle` whose `area` calls the
//!   same-file helper `square`.
//! - `a/report`: `describe` calls the same-file `label` and the same-dir
//!   `square`.
//! - `b/app`: imports from `a`; `main` calls the same-file `run`; `run` calls
//!   `describe` (cross-dir) and constructs a `Circle`.
//!
//! Asserted: same-file / same-dir calls, imports, import- / package- /
//! module-backed cross-dir calls, extends / implements edges and
//! instantiates edges (`run` constructs a `Circle`).
#![allow(clippy::expect_used)]

mod common;

use std::collections::BTreeSet;
use std::path::Path;

use common::{index_project, query_set};
use coraline::tools;
use serde_json::{Value, json};

struct Fixture {
    lang: &'static str,
    files: &'static [(&'static str, &'static str)],
    /// Resolved same-file / same-dir `caller -> callee` call edges (PR 1).
    calls: &'static [&'static str],
    /// `file -> import` edges (PR 1).
    imports: &'static [&'static str],
    /// Resolved cross-dir `caller -> callee` call edges (PR 2).
    cross_dir_calls: &'static [&'static str],
    /// `sub -> super` extends / implements edges (PR 3).
    inherits: &'static [&'static str],
    /// `caller -> class` instantiates edges (PR 3).
    instantiates: &'static [&'static str],
}

const KOTLIN: Fixture = Fixture {
    lang: "kotlin",
    files: &[
        (
            "a/Shapes.kt",
            "package app.a\n\
             \n\
             abstract class Shape {\n\
             \x20   abstract fun area(): Double\n\
             }\n\
             \n\
             class Circle(val r: Double) : Shape() {\n\
             \x20   override fun area(): Double = square(r) * 3.14\n\
             }\n\
             \n\
             fun square(x: Double): Double = x * x\n",
        ),
        (
            "a/Report.kt",
            "package app.a\n\
             \n\
             fun describe(x: Double): String = label(square(x))\n\
             \n\
             fun label(v: Double): String = \"area=\" + v\n",
        ),
        (
            "b/App.kt",
            "package app.b\n\
             \n\
             import app.a.Circle\n\
             import app.a.describe\n\
             \n\
             fun run() {\n\
             \x20   describe(2.0)\n\
             \x20   Circle(1.0)\n\
             }\n\
             \n\
             fun main() {\n\
             \x20   run()\n\
             }\n",
        ),
    ],
    calls: &[
        "area -> square",
        "describe -> label",
        "describe -> square",
        "main -> run",
    ],
    imports: &["App.kt -> Circle", "App.kt -> describe"],
    cross_dir_calls: &["run -> describe"],
    inherits: &["Circle -> Shape"],
    instantiates: &["run -> Circle"],
};

const JAVA: Fixture = Fixture {
    lang: "java",
    files: &[
        (
            "a/Shape.java",
            "package app.a;\n\
             \n\
             public abstract class Shape {\n\
             \x20   public abstract double area();\n\
             }\n",
        ),
        (
            "a/Circle.java",
            "package app.a;\n\
             \n\
             public class Circle extends Shape {\n\
             \x20   private final double r;\n\
             \x20   public Circle(double r) { this.r = r; }\n\
             \x20   public double area() { return square(r) * 3.14; }\n\
             \x20   static double square(double x) { return x * x; }\n\
             }\n",
        ),
        (
            "a/Report.java",
            "package app.a;\n\
             \n\
             public class Report {\n\
             \x20   public static String describe(double x) { return label(Circle.square(x)); }\n\
             \x20   static String label(double v) { return \"area=\" + v; }\n\
             }\n",
        ),
        (
            "b/App.java",
            "package app.b;\n\
             \n\
             import app.a.Circle;\n\
             import app.a.Report;\n\
             \n\
             public class App {\n\
             \x20   static void run() { Report.describe(2.0); new Circle(1.0); }\n\
             \x20   public static void main(String[] args) { run(); }\n\
             }\n",
        ),
    ],
    calls: &[
        "area -> square",
        "describe -> label",
        "describe -> square",
        "main -> run",
    ],
    imports: &["App.java -> Circle", "App.java -> Report"],
    cross_dir_calls: &["run -> describe"],
    inherits: &["Circle -> Shape"],
    instantiates: &["run -> Circle"],
};

const SWIFT: Fixture = Fixture {
    lang: "swift",
    files: &[
        // An Xcode project makes the fixture one Swift module.
        ("App.xcodeproj/project.pbxproj", "// !$*UTF8*$!\n"),
        (
            "a/Shapes.swift",
            "class Shape {\n\
             \x20   func area() -> Double { return 0 }\n\
             }\n\
             \n\
             class Circle: Shape {\n\
             \x20   let r: Double\n\
             \x20   init(r: Double) { self.r = r }\n\
             \x20   override func area() -> Double { return square(r) * 3.14 }\n\
             }\n\
             \n\
             func square(_ x: Double) -> Double { return x * x }\n",
        ),
        (
            "a/Report.swift",
            "func describe(_ x: Double) -> String { return label(square(x)) }\n\
             \n\
             func label(_ v: Double) -> String { return \"area=\\(v)\" }\n",
        ),
        (
            "b/App.swift",
            "import Foundation\n\
             import Shapes\n\
             \n\
             func run() {\n\
             \x20   _ = describe(2.0)\n\
             \x20   _ = Circle(r: 1.0)\n\
             }\n\
             \n\
             func main() { run() }\n",
        ),
    ],
    calls: &[
        "area -> square",
        "describe -> label",
        "describe -> square",
        "main -> run",
    ],
    imports: &["App.swift -> Foundation", "App.swift -> Shapes"],
    // Same Swift module, so the call is legal without an import.
    cross_dir_calls: &["run -> describe"],
    inherits: &["Circle -> Shape"],
    instantiates: &["run -> Circle"],
};

const GO: Fixture = Fixture {
    lang: "go",
    files: &[
        (
            "a/shapes.go",
            "package a\n\
             \n\
             type Shape interface {\n\
             \x20   Area() float64\n\
             }\n\
             \n\
             type Circle struct {\n\
             \x20   R float64\n\
             }\n\
             \n\
             func (c Circle) Area() float64 { return Square(c.R) * 3.14 }\n\
             \n\
             func Square(x float64) float64 { return x * x }\n",
        ),
        (
            "a/report.go",
            "package a\n\
             \n\
             import \"fmt\"\n\
             \n\
             func Describe(x float64) string { return Label(Square(x)) }\n\
             \n\
             func Label(v float64) string { return fmt.Sprint(v) }\n",
        ),
        (
            "b/app.go",
            "package main\n\
             \n\
             import (\n\
             \x20   \"example.com/app/a\"\n\
             )\n\
             \n\
             func Run() {\n\
             \x20   a.Describe(2.0)\n\
             \x20   _ = a.Circle{R: 1.0}\n\
             }\n\
             \n\
             func main() { Run() }\n",
        ),
    ],
    calls: &[
        "Area -> Square",
        "Describe -> Label",
        "Describe -> Square",
        "main -> Run",
    ],
    imports: &["report.go -> fmt", "app.go -> a"],
    cross_dir_calls: &["Run -> Describe"],
    // Go interfaces are satisfied implicitly: no inheritance edge expected.
    inherits: &[],
    instantiates: &["Run -> Circle"],
};

const PYTHON: Fixture = Fixture {
    lang: "python",
    files: &[
        (
            "a/shapes.py",
            "class Shape:\n\
             \x20   def area(self):\n\
             \x20       raise NotImplementedError\n\
             \n\
             \n\
             class Circle(Shape):\n\
             \x20   def __init__(self, r):\n\
             \x20       self.r = r\n\
             \n\
             \x20   def area(self):\n\
             \x20       return square(self.r) * 3.14\n\
             \n\
             \n\
             def square(x):\n\
             \x20   return x * x\n",
        ),
        (
            "a/report.py",
            "from .shapes import square\n\
             \n\
             \n\
             def describe(x):\n\
             \x20   return label(square(x))\n\
             \n\
             \n\
             def label(v):\n\
             \x20   return f\"area={v}\"\n",
        ),
        (
            "b/app.py",
            "from a.report import describe\n\
             from a.shapes import Circle\n\
             \n\
             \n\
             def run():\n\
             \x20   describe(2.0)\n\
             \x20   Circle(1.0)\n\
             \n\
             \n\
             def main():\n\
             \x20   run()\n",
        ),
    ],
    calls: &[
        "area -> square",
        "describe -> label",
        "describe -> square",
        "main -> run",
    ],
    imports: &[
        "report.py -> square",
        "app.py -> describe",
        "app.py -> Circle",
    ],
    cross_dir_calls: &["run -> describe"],
    inherits: &["Circle -> Shape"],
    instantiates: &["run -> Circle"],
};

const TYPESCRIPT: Fixture = Fixture {
    lang: "typescript",
    files: &[
        (
            "a/shapes.ts",
            "export abstract class Shape {\n\
             \x20 abstract area(): number;\n\
             }\n\
             \n\
             export class Circle extends Shape {\n\
             \x20 constructor(private r: number) {\n\
             \x20   super();\n\
             \x20 }\n\
             \x20 area(): number {\n\
             \x20   return square(this.r) * 3.14;\n\
             \x20 }\n\
             }\n\
             \n\
             export function square(x: number): number {\n\
             \x20 return x * x;\n\
             }\n",
        ),
        (
            "a/report.ts",
            "import { square } from './shapes';\n\
             \n\
             export function describe(x: number): string {\n\
             \x20 return label(square(x));\n\
             }\n\
             \n\
             function label(v: number): string {\n\
             \x20 return `area=${v}`;\n\
             }\n",
        ),
        (
            "b/app.ts",
            "import { describe } from '../a/report';\n\
             import { Circle } from '../a/shapes';\n\
             \n\
             function run(): void {\n\
             \x20 describe(2);\n\
             \x20 new Circle(1);\n\
             }\n\
             \n\
             export function main(): void {\n\
             \x20 run();\n\
             }\n",
        ),
    ],
    calls: &[
        "area -> square",
        "describe -> label",
        "describe -> square",
        "main -> run",
    ],
    imports: &[
        "report.ts -> square",
        "app.ts -> describe",
        "app.ts -> Circle",
    ],
    cross_dir_calls: &["run -> describe"],
    inherits: &["Circle -> Shape"],
    instantiates: &["run -> Circle"],
};

const CSHARP: Fixture = Fixture {
    lang: "csharp",
    files: &[
        (
            "a/Shapes.cs",
            "namespace App.A\n\
             {\n\
             \x20   public abstract class Shape\n\
             \x20   {\n\
             \x20       public abstract double Area();\n\
             \x20   }\n\
             \n\
             \x20   public class Circle : Shape\n\
             \x20   {\n\
             \x20       private readonly double r;\n\
             \x20       public Circle(double r) { this.r = r; }\n\
             \x20       public override double Area() { return Geometry.Square(r) * 3.14; }\n\
             \x20   }\n\
             \n\
             \x20   public static class Geometry\n\
             \x20   {\n\
             \x20       public static double Square(double x) { return x * x; }\n\
             \x20   }\n\
             }\n",
        ),
        (
            "a/Report.cs",
            "using System;\n\
             \n\
             namespace App.A\n\
             {\n\
             \x20   public static class Report\n\
             \x20   {\n\
             \x20       public static string Describe(double x) { return Label(Geometry.Square(x)); }\n\
             \x20       static string Label(double v) { return \"area=\" + v; }\n\
             \x20   }\n\
             }\n",
        ),
        (
            "b/App.cs",
            "using App.A;\n\
             \n\
             namespace App.B\n\
             {\n\
             \x20   public static class Program\n\
             \x20   {\n\
             \x20       static void Run() { Report.Describe(2.0); new Circle(1.0); }\n\
             \x20       public static void Main() { Run(); }\n\
             \x20   }\n\
             }\n",
        ),
    ],
    calls: &[
        "Area -> Square",
        "Describe -> Label",
        "Describe -> Square",
        "Main -> Run",
    ],
    imports: &["Report.cs -> System", "App.cs -> A"],
    cross_dir_calls: &["Run -> Describe"],
    inherits: &["Circle -> Shape"],
    instantiates: &["Run -> Circle"],
};

const RUST: Fixture = Fixture {
    lang: "rust",
    files: &[
        (
            "src/a/shapes.rs",
            "pub trait Shape {\n\
             \x20   fn area(&self) -> f64;\n\
             }\n\
             \n\
             pub struct Circle {\n\
             \x20   pub r: f64,\n\
             }\n\
             \n\
             impl Shape for Circle {\n\
             \x20   fn area(&self) -> f64 {\n\
             \x20       square(self.r) * 3.14\n\
             \x20   }\n\
             }\n\
             \n\
             pub fn square(x: f64) -> f64 {\n\
             \x20   x * x\n\
             }\n",
        ),
        (
            "src/a/report.rs",
            "use super::shapes::square;\n\
             \n\
             pub fn describe(x: f64) -> String {\n\
             \x20   label(square(x))\n\
             }\n\
             \n\
             fn label(v: f64) -> String {\n\
             \x20   format!(\"area={v}\")\n\
             }\n",
        ),
        (
            "src/b/app.rs",
            "use crate::a::report::describe;\n\
             use crate::a::shapes::Circle;\n\
             \n\
             fn run() {\n\
             \x20   describe(2.0);\n\
             \x20   let _c = Circle { r: 1.0 };\n\
             }\n\
             \n\
             pub fn main() {\n\
             \x20   run();\n\
             }\n",
        ),
    ],
    calls: &[
        "area -> square",
        "describe -> label",
        "describe -> square",
        "main -> run",
    ],
    imports: &[
        "report.rs -> square",
        "app.rs -> describe",
        "app.rs -> Circle",
    ],
    cross_dir_calls: &["run -> describe"],
    inherits: &["Circle -> Shape"],
    instantiates: &["run -> Circle"],
};

const C_CPP: Fixture = Fixture {
    lang: "c/c++",
    files: &[
        (
            "a/shapes.c",
            "#include \"shapes.h\"\n\
             \n\
             double square(double x) { return x * x; }\n",
        ),
        (
            "a/report.c",
            "#include <stdio.h>\n\
             #include \"shapes.h\"\n\
             \n\
             static const char *label(double v) { return \"area\"; }\n\
             \n\
             const char *describe(double x) { return label(square(x)); }\n",
        ),
        (
            "a/circle.cpp",
            "#include \"shapes.h\"\n\
             \n\
             class Shape {\n\
             public:\n\
             \x20   virtual double area() = 0;\n\
             };\n\
             \n\
             class Circle : public Shape {\n\
             public:\n\
             \x20   double r;\n\
             \x20   double area() override { return square(r) * 3.14; }\n\
             };\n",
        ),
        (
            "b/app.cpp",
            "#include \"../a/report.h\"\n\
             #include \"../a/circle.h\"\n\
             \n\
             static void run() {\n\
             \x20   describe(2.0);\n\
             \x20   Shape *s = new Circle();\n\
             }\n\
             \n\
             int main() {\n\
             \x20   run();\n\
             \x20   return 0;\n\
             }\n",
        ),
    ],
    calls: &[
        "area -> square",
        "describe -> label",
        "describe -> square",
        "main -> run",
    ],
    imports: &[
        "report.c -> <stdio.h>",
        "report.c -> shapes",
        "app.cpp -> report",
    ],
    cross_dir_calls: &["run -> describe"],
    inherits: &["Circle -> Shape"],
    instantiates: &["run -> Circle"],
};

const RUBY: Fixture = Fixture {
    lang: "ruby",
    files: &[
        (
            "a/shapes.rb",
            "class Shape\n\
             \x20 def area\n\
             \x20   raise NotImplementedError\n\
             \x20 end\n\
             end\n\
             \n\
             class Circle < Shape\n\
             \x20 def initialize(r)\n\
             \x20   @r = r\n\
             \x20 end\n\
             \n\
             \x20 def area\n\
             \x20   square(@r) * 3.14\n\
             \x20 end\n\
             end\n\
             \n\
             def square(x)\n\
             \x20 x * x\n\
             end\n",
        ),
        (
            "a/report.rb",
            "require_relative 'shapes'\n\
             \n\
             def describe(x)\n\
             \x20 label(square(x))\n\
             end\n\
             \n\
             def label(v)\n\
             \x20 \"area=#{v}\"\n\
             end\n",
        ),
        (
            "b/app.rb",
            "require_relative '../a/report'\n\
             require_relative '../a/shapes'\n\
             \n\
             def run\n\
             \x20 describe(2.0)\n\
             \x20 Circle.new(1.0)\n\
             end\n\
             \n\
             def main\n\
             \x20 run()\n\
             end\n",
        ),
    ],
    calls: &[
        "area -> square",
        "describe -> label",
        "describe -> square",
        "main -> run",
    ],
    imports: &["report.rb -> shapes", "app.rb -> report"],
    cross_dir_calls: &["run -> describe"],
    inherits: &["Circle -> Shape"],
    instantiates: &["run -> Circle"],
};

const PHP: Fixture = Fixture {
    lang: "php",
    files: &[
        (
            "a/Shapes.php",
            "<?php\n\
             namespace App\\A;\n\
             \n\
             abstract class Shape {\n\
             \x20   abstract public function area(): float;\n\
             }\n\
             \n\
             class Circle extends Shape {\n\
             \x20   public function __construct(private float $r) {}\n\
             \x20   public function area(): float { return square($this->r) * 3.14; }\n\
             }\n\
             \n\
             function square(float $x): float { return $x * $x; }\n",
        ),
        (
            "a/Report.php",
            "<?php\n\
             namespace App\\A;\n\
             \n\
             function describe(float $x): string { return label(square($x)); }\n\
             \n\
             function label(float $v): string { return \"area=\" . $v; }\n",
        ),
        (
            "b/App.php",
            "<?php\n\
             namespace App\\B;\n\
             \n\
             use App\\A\\Circle;\n\
             use function App\\A\\describe;\n\
             \n\
             function run(): void { describe(2.0); new Circle(1.0); }\n\
             \n\
             function main(): void { run(); }\n",
        ),
    ],
    calls: &[
        "area -> square",
        "describe -> label",
        "describe -> square",
        "main -> run",
    ],
    imports: &["App.php -> Circle", "App.php -> describe"],
    cross_dir_calls: &["run -> describe"],
    inherits: &["Circle -> Shape"],
    instantiates: &["run -> Circle"],
};

const FIXTURES: &[Fixture] = &[
    KOTLIN, JAVA, SWIFT, GO, PYTHON, TYPESCRIPT, CSHARP, RUST, C_CPP, RUBY, PHP,
];

/// `source -> target` names of stored edges of `kind`.
fn edge_pairs(project_path: &Path, kind: &str) -> BTreeSet<String> {
    query_set(
        project_path,
        &format!(
            "SELECT s.name || ' -> ' || t.name FROM edges e
               JOIN nodes s ON s.id = e.source JOIN nodes t ON t.id = e.target
              WHERE e.kind = '{kind}'"
        ),
    )
}

/// Missing entries of `expected` in `actual`, prefixed with the language.
fn missing(lang: &str, what: &str, actual: &BTreeSet<String>, expected: &[&str]) -> Vec<String> {
    expected
        .iter()
        .filter(|e| !actual.contains(**e))
        .map(|e| format!("{lang}: missing {what} `{e}` in {actual:?}"))
        .collect()
}

/// Index every fixture and collect the failures reported by `check`, so one
/// run shows every failing language at once.
fn check_all(check: impl Fn(&Fixture, &Path) -> Vec<String>) {
    let failures: Vec<String> = FIXTURES
        .iter()
        .flat_map(|fixture| {
            let temp = index_project(fixture.files);
            check(fixture, temp.path())
        })
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Names returned by `coraline_callers` / `coraline_callees` under `key`.
fn tool_names(project_path: &Path, tool: &str, key: &str, node_id: &str) -> BTreeSet<String> {
    tool_names_with(
        project_path,
        tool,
        key,
        json!({ "node_id": node_id, "limit": 50 }),
    )
}

fn tool_names_with(project_path: &Path, tool: &str, key: &str, params: Value) -> BTreeSet<String> {
    let output = tools::create_default_registry(project_path)
        .execute(tool, params)
        .expect("tool call failed");
    output
        .get(key)
        .and_then(Value::as_array)
        .expect("tool output should contain an array")
        .iter()
        .filter_map(|item| item.get("name").and_then(Value::as_str))
        .map(str::to_string)
        .collect()
}

#[test]
fn same_file_and_same_dir_calls_are_stored() {
    check_all(|fixture, path| {
        missing(
            fixture.lang,
            "calls edge",
            &edge_pairs(path, "calls"),
            fixture.calls,
        )
    });
}

#[test]
fn imports_are_stored() {
    check_all(|fixture, path| {
        let actual = edge_pairs(path, "imports");
        let mut failures = missing(fixture.lang, "imports edge", &actual, fixture.imports);
        if fixture.imports.is_empty() && !actual.is_empty() {
            failures.push(format!("{}: unexpected imports {actual:?}", fixture.lang));
        }
        failures
    });
}

#[test]
fn callers_and_callees_return_stored_calls() {
    check_all(|fixture, path| {
        let conn = coraline::db::open_database(path).expect("Failed to open database");
        let mut failures = Vec::new();
        for pair in fixture.calls {
            let (source_name, target_name) = pair.split_once(" -> ").expect("pair format");
            let ids: Option<(String, String)> = conn
                .query_row(
                    "SELECT e.source, e.target FROM edges e
                       JOIN nodes s ON s.id = e.source JOIN nodes t ON t.id = e.target
                      WHERE e.kind = 'calls' AND s.name = ?1 AND t.name = ?2",
                    [source_name, target_name],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .ok();
            let Some((source, target)) = ids else {
                failures.push(format!("{}: no calls edge `{pair}`", fixture.lang));
                continue;
            };
            let outgoing = tool_names(path, "coraline_callees", "callees", &source);
            if !outgoing.contains(target_name) {
                failures.push(format!(
                    "{}: callees of {source_name} lack {target_name}: {outgoing:?}",
                    fixture.lang
                ));
            }
            let incoming = tool_names(path, "coraline_callers", "callers", &target);
            if !incoming.contains(source_name) {
                failures.push(format!(
                    "{}: callers of {target_name} lack {source_name}: {incoming:?}",
                    fixture.lang
                ));
            }
        }
        failures
    });
}

#[test]
fn cross_dir_calls_are_stored() {
    check_all(|fixture, path| {
        missing(
            fixture.lang,
            "cross-dir calls edge",
            &edge_pairs(path, "calls"),
            fixture.cross_dir_calls,
        )
    });
}

#[test]
fn inheritance_edges_are_stored() {
    check_all(|fixture, path| {
        let mut actual = edge_pairs(path, "extends");
        actual.extend(edge_pairs(path, "implements"));
        missing(
            fixture.lang,
            "extends/implements edge",
            &actual,
            fixture.inherits,
        )
    });
}

/// `coraline_callers` / `coraline_find_references` filtered by
/// `edge_kind` return exactly the sources of the stored edges of that kind.
#[test]
fn type_edges_are_retrievable_by_edge_kind() {
    check_all(|fixture, path| {
        let conn = coraline::db::open_database(path).expect("Failed to open database");
        let mut failures = Vec::new();
        let expectations = fixture
            .inherits
            .iter()
            .map(|pair| (pair, ("extends", "implements")))
            .chain(
                fixture
                    .instantiates
                    .iter()
                    .map(|pair| (pair, ("instantiates", "instantiates"))),
            );
        for (pair, (kind_a, kind_b)) in expectations {
            let (source_name, target_name) = pair.split_once(" -> ").expect("pair format");
            let found: Option<(String, String)> = conn
                .query_row(
                    "SELECT e.target, e.kind FROM edges e
                       JOIN nodes s ON s.id = e.source JOIN nodes t ON t.id = e.target
                      WHERE s.name = ?1 AND t.name = ?2 AND e.kind IN (?3, ?4)",
                    [source_name, target_name, kind_a, kind_b],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .ok();
            let Some((target, kind)) = found else {
                failures.push(format!("{}: no edge `{pair}`", fixture.lang));
                continue;
            };
            let expected = BTreeSet::from([source_name.to_string()]);
            for (tool, key) in [
                ("coraline_callers", "callers"),
                ("coraline_find_references", "references"),
            ] {
                let names = tool_names_with(
                    path,
                    tool,
                    key,
                    json!({ "node_id": target, "edge_kind": kind, "limit": 50 }),
                );
                if names != expected {
                    failures.push(format!(
                        "{}: {tool} {kind} of {target_name}: {names:?}, expected {expected:?}",
                        fixture.lang
                    ));
                }
            }
        }
        failures
    });
}

#[test]
fn instantiates_edges_are_stored() {
    check_all(|fixture, path| {
        missing(
            fixture.lang,
            "instantiates edge",
            &edge_pairs(path, "instantiates"),
            fixture.instantiates,
        )
    });
}

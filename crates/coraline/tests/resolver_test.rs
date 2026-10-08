//! Reference resolver tests: which cross-file `Calls` refs become edges.
//!
//! The resolver must only link calls across directories when the link is
//! backed by an import or by same-package / same-namespace membership. A
//! global name-only fallback produced false edges between unrelated crates
//! (upstream issue #43: `raccoon-agent::heartbeat()` →
//! `raccoon-frontend/.../client.rs::post`); the `no_import_*` tests guard
//! against reintroducing it.
#![allow(clippy::expect_used)]

mod common;

use std::collections::BTreeSet;
use std::path::Path;

use common::{index_project, query_set};

/// `caller -> callee @ callee_file` for every stored calls edge.
fn call_edges(project_path: &Path) -> BTreeSet<String> {
    query_set(
        project_path,
        "SELECT s.name || ' -> ' || t.name || ' @ ' || replace(t.file_path, '\\', '/')
           FROM edges e
           JOIN nodes s ON s.id = e.source JOIN nodes t ON t.id = e.target
          WHERE e.kind = 'calls'",
    )
}

/// Calls edges whose caller and callee live in different top-level
/// directories (`agent/...` → `frontend/...`).
fn cross_root_calls(project_path: &Path) -> Vec<String> {
    let rows = query_set(
        project_path,
        "SELECT replace(s.file_path, '\\', '/') || ' ' || s.name || ' -> '
                || replace(t.file_path, '\\', '/') || ' ' || t.name
           FROM edges e
           JOIN nodes s ON s.id = e.source JOIN nodes t ON t.id = e.target
          WHERE e.kind = 'calls'",
    );
    rows.into_iter()
        .filter(|row| {
            let mut parts = row.split(' ');
            let source_root = parts.next().and_then(|p| p.split('/').next());
            let target_root = parts.nth(2).and_then(|p| p.split('/').next());
            source_root != target_root
        })
        .collect()
}

fn assert_no_cross_root_calls(files: &[(&str, &str)]) {
    let temp = index_project(files);
    let crossing = cross_root_calls(temp.path());
    assert!(
        crossing.is_empty(),
        "unrelated roots must not be linked by name only: {crossing:?}\nall calls: {:?}",
        call_edges(temp.path())
    );
}

/// Mirror of #43: the agent crate calls `post` / `new` / `remove` on a
/// third-party client; the unrelated frontend crate defines free functions
/// with the same names. The agent's own crate-relative import must not pull
/// in the frontend either.
#[test]
fn no_import_no_edge_rust_issue_43() {
    assert_no_cross_root_calls(&[
        (
            "raccoon-agent/src/heartbeat.rs",
            "use reqwest::Client;\n\
             use crate::settings::config::load;\n\
             \n\
             pub fn heartbeat() {\n\
             \x20   let client = Client::new();\n\
             \x20   let url = load();\n\
             \x20   client.post(url).send();\n\
             \x20   remove(url);\n\
             }\n",
        ),
        (
            "raccoon-agent/src/settings/config.rs",
            "pub fn load() -> u32 {\n\
             \x20   0\n\
             }\n",
        ),
        (
            "raccoon-frontend/src/api/client.rs",
            "pub fn post(url: u32) {}\n\
             pub fn new() {}\n\
             pub fn remove(url: u32) {}\n\
             pub fn send() {}\n",
        ),
        (
            "raccoon-frontend/src/config.rs",
            "pub fn load() -> u32 {\n\
             \x20   1\n\
             }\n",
        ),
    ]);
}

#[test]
fn no_import_no_edge_go() {
    assert_no_cross_root_calls(&[
        (
            "agent/heartbeat.go",
            "package agent\n\
             \n\
             import (\n\
             \x20   \"net/http\"\n\
             \x20   \"example.com/agent/util\"\n\
             )\n\
             \n\
             func Heartbeat(client *http.Client) {\n\
             \x20   client.Post(util.URL(), \"\", nil)\n\
             \x20   Remove()\n\
             }\n",
        ),
        (
            "agent/util/url.go",
            "package util\n\
             \n\
             func URL() string { return \"\" }\n",
        ),
        (
            "frontend/api/client.go",
            "package api\n\
             \n\
             func Post() {}\n\
             func Remove() {}\n\
             func URL() string { return \"\" }\n",
        ),
    ]);
}

#[test]
fn no_import_no_edge_python() {
    assert_no_cross_root_calls(&[
        (
            "agent/heartbeat.py",
            "import requests\n\
             from .util import load\n\
             \n\
             \n\
             def heartbeat():\n\
             \x20   requests.post(load())\n\
             \x20   post()\n\
             \x20   remove()\n",
        ),
        ("agent/util.py", "def load():\n    return ''\n"),
        (
            "frontend/api/client.py",
            "def post():\n    pass\n\n\ndef remove():\n    pass\n",
        ),
        ("frontend/util.py", "def load():\n    return ''\n"),
    ]);
}

#[test]
fn no_import_no_edge_typescript() {
    assert_no_cross_root_calls(&[
        (
            "agent/heartbeat.ts",
            "import axios from 'axios';\n\
             import { load } from './util';\n\
             \n\
             export function heartbeat(): void {\n\
             \x20 axios.post(load());\n\
             \x20 post();\n\
             }\n",
        ),
        (
            "agent/util.ts",
            "export function load(): string { return ''; }\n",
        ),
        (
            "frontend/api/client.ts",
            "export function post(): void {}\n",
        ),
        (
            "frontend/util.ts",
            "export function load(): string { return ''; }\n",
        ),
    ]);
}

#[test]
fn no_import_no_edge_kotlin() {
    assert_no_cross_root_calls(&[
        (
            "agent/Heartbeat.kt",
            "package raccoon.agent\n\
             \n\
             import io.ktor.client.HttpClient\n\
             \n\
             fun heartbeat(client: HttpClient) {\n\
             \x20   client.post(\"/hb\")\n\
             \x20   post(\"/hb\")\n\
             \x20   Client.remove()\n\
             }\n",
        ),
        (
            "frontend/Client.kt",
            "package raccoon.frontend\n\
             \n\
             fun post(url: String) {}\n\
             \n\
             object Client {\n\
             \x20   fun remove() {}\n\
             }\n",
        ),
    ]);
}

#[test]
fn no_import_no_edge_java() {
    assert_no_cross_root_calls(&[
        (
            "agent/Heartbeat.java",
            "package raccoon.agent;\n\
             \n\
             import java.net.http.HttpClient;\n\
             \n\
             public class Heartbeat {\n\
             \x20   void beat(HttpClient client) { client.post(); Client.remove(); }\n\
             }\n",
        ),
        (
            "frontend/Client.java",
            "package raccoon.frontend;\n\
             \n\
             public class Client {\n\
             \x20   public void post() {}\n\
             \x20   public static void remove() {}\n\
             }\n",
        ),
    ]);
}

#[test]
fn no_import_no_edge_csharp() {
    assert_no_cross_root_calls(&[
        (
            "agent/Heartbeat.cs",
            "using System.Net.Http;\n\
             \n\
             namespace Raccoon.Agent\n\
             {\n\
             \x20   public class Heartbeat\n\
             \x20   {\n\
             \x20       public void Beat(HttpClient client) { client.Post(); Client.Remove(); }\n\
             \x20   }\n\
             }\n",
        ),
        (
            "frontend/Client.cs",
            "namespace Raccoon.Frontend\n\
             {\n\
             \x20   public static class Client\n\
             \x20   {\n\
             \x20       public static void Post() {}\n\
             \x20       public static void Remove() {}\n\
             \x20   }\n\
             }\n",
        ),
    ]);
}

#[test]
fn no_import_no_edge_php() {
    assert_no_cross_root_calls(&[
        (
            "agent/Heartbeat.php",
            "<?php\n\
             namespace Raccoon\\Agent;\n\
             \n\
             use GuzzleHttp\\Client;\n\
             \n\
             function heartbeat(Client $client): void { $client->post('/hb'); post(); }\n",
        ),
        (
            "frontend/Client.php",
            "<?php\n\
             namespace Raccoon\\Frontend;\n\
             \n\
             function post(): void {}\n",
        ),
    ]);
}

#[test]
fn no_import_no_edge_c() {
    assert_no_cross_root_calls(&[
        (
            "agent/heartbeat.c",
            "#include <curl/curl.h>\n\
             #include \"client.h\"\n\
             \n\
             void heartbeat(void) { post(\"/hb\"); }\n",
        ),
        ("agent/client.h", "void post(const char *url);\n"),
        ("frontend/client.c", "void post(const char *url) {}\n"),
    ]);
}

/// Index `files` and assert every `caller -> callee @ callee_file` edge in
/// `expected` is stored.
fn assert_calls(files: &[(&str, &str)], expected: &[&str]) {
    let temp = index_project(files);
    let actual = call_edges(temp.path());
    let missing: Vec<_> = expected.iter().filter(|e| !actual.contains(**e)).collect();
    assert!(missing.is_empty(), "missing {missing:?} in {actual:#?}");
    // Decoys: the same caller/callee pair must not also resolve elsewhere.
    let decoys: Vec<_> = actual
        .iter()
        .filter(|a| {
            expected
                .iter()
                .any(|e| *a != e && a.split(" @ ").next() == e.split(" @ ").next())
        })
        .collect();
    assert!(decoys.is_empty(), "unexpected {decoys:?} in {actual:#?}");
}

/// Same project as the #43 mirror: the crate-relative import resolves to the
/// agent's own `settings/config.rs`, not the frontend's `config.rs`.
#[test]
fn rust_crate_import_resolves_within_the_crate() {
    assert_calls(
        &[
            (
                "raccoon-agent/src/beat/heartbeat.rs",
                "use crate::settings::config::load;\n\
                 use super::super::util::helper as aliased;\n\
                 \n\
                 pub fn heartbeat() {\n\
                 \x20   load();\n\
                 \x20   aliased();\n\
                 }\n",
            ),
            (
                "raccoon-agent/src/settings/config.rs",
                "pub fn load() -> u32 {\n    0\n}\n",
            ),
            ("raccoon-agent/src/util.rs", "pub fn helper() {}\n"),
            (
                "raccoon-frontend/src/config.rs",
                "pub fn load() -> u32 {\n    1\n}\n",
            ),
            ("raccoon-frontend/src/util.rs", "pub fn helper() {}\n"),
        ],
        &[
            "heartbeat -> load @ raccoon-agent/src/settings/config.rs",
            "heartbeat -> helper @ raccoon-agent/src/util.rs",
        ],
    );
}

#[test]
fn typescript_relative_import_resolves_against_the_importing_file() {
    assert_calls(
        &[
            (
                "agent/src/heartbeat.ts",
                "import { load } from '../lib/util';\n\
                 import { post as send } from './net.js';\n\
                 \n\
                 export function heartbeat(): void {\n\
                 \x20 send(load());\n\
                 }\n",
            ),
            (
                "agent/lib/util.ts",
                "export function load(): string { return ''; }\n",
            ),
            (
                "agent/src/net.ts",
                "export function post(url: string): void {}\n",
            ),
            (
                "frontend/lib/util.ts",
                "export function load(): string { return ''; }\n",
            ),
        ],
        &[
            "heartbeat -> load @ agent/lib/util.ts",
            "heartbeat -> post @ agent/src/net.ts",
        ],
    );
}

#[test]
fn python_dotted_and_relative_imports_resolve() {
    assert_calls(
        &[
            (
                "agent/app/heartbeat.py",
                "from agent.core.util import load\n\
                 from ..net import post\n\
                 \n\
                 \n\
                 def heartbeat():\n\
                 \x20   post(load())\n",
            ),
            ("agent/core/util.py", "def load():\n    return ''\n"),
            ("agent/net/__init__.py", "def post(url):\n    pass\n"),
            ("frontend/core/util.py", "def load():\n    return ''\n"),
        ],
        &[
            "heartbeat -> load @ agent/core/util.py",
            "heartbeat -> post @ agent/net/__init__.py",
        ],
    );
}

/// `caller -> qualifier.callee` for unresolved call refs that have a qualifier.
fn qualified_refs(project_path: &Path) -> BTreeSet<String> {
    query_set(
        project_path,
        "SELECT n.name || ' -> ' || u.qualifier || '.' || u.reference_name
           FROM unresolved_refs u JOIN nodes n ON n.id = u.from_node_id
          WHERE u.qualifier IS NOT NULL",
    )
}

#[test]
fn call_qualifiers_are_stored() {
    let temp = index_project(&[
        (
            "j/A.java",
            "class A { void j() { Ext.run(); app.ext.Ext.go(); } }\n",
        ),
        ("k/a.kt", "fun k() { Ext.run(); app.ext.go() }\n"),
        ("s/a.swift", "func s() { Ext.run() }\n"),
        ("g/a.go", "package g\n\nfunc G() { ext.Run() }\n"),
        ("p/a.py", "def p():\n    ext.sub.run()\n"),
        ("t/a.ts", "function t() { ext.run(); this.go(); }\n"),
        ("r/a.rs", "fn r() { ext::sub::run(); Ext::go(); }\n"),
        ("c/a.cs", "class C { void Cs() { Ext.Run(); } }\n"),
        ("c/a.cpp", "void cpp() { ext::run(); }\n"),
        (
            "h/a.php",
            "<?php\nfunction php() { Ext::run(); \\App\\Ext::go(); $x->no(); }\n",
        ),
        ("b/a.rb", "def rb\n  Ext::Sub.run()\nend\n"),
    ]);
    let refs = qualified_refs(temp.path());
    for expected in [
        "j -> Ext.run",
        "j -> app.ext.Ext.go",
        "k -> Ext.run",
        "k -> app.ext.go",
        "s -> Ext.run",
        "G -> ext.Run",
        "p -> ext.sub.run",
        "t -> ext.run",
        "t -> this.go",
        "r -> ext::sub.run",
        "r -> Ext.go",
        "Cs -> Ext.Run",
        "cpp -> ext.run",
        "php -> Ext.run",
        "php -> \\App\\Ext.go",
        "rb -> Ext::Sub.run",
    ] {
        assert!(refs.contains(expected), "missing {expected} in {refs:#?}");
    }
    assert!(
        !refs.iter().any(|r| r.contains("$x")),
        "expression receivers have no qualifier: {refs:#?}"
    );
}

/// `Util.format(..)` with `Util` imported resolves into the imported class,
/// not to the same-dir `format` nor to an unrelated `Util`.
#[test]
fn java_qualifier_resolves_through_class_import() {
    assert_calls(
        &[
            (
                "src/main/java/com/example/app/App.java",
                "package com.example.app;\n\
                 \n\
                 import com.example.util.Util;\n\
                 \n\
                 public class App {\n\
                 \x20   String format(int x) { return Util.format(x); }\n\
                 }\n",
            ),
            (
                "src/main/java/com/example/app/Format.java",
                "package com.example.app;\n\
                 \n\
                 class Format {\n\
                 \x20   static String format(int x) { return \"\"; }\n\
                 }\n",
            ),
            (
                "src/main/java/com/example/util/Util.java",
                "package com.example.util;\n\
                 \n\
                 public class Util {\n\
                 \x20   public static String format(int x) { return \"\"; }\n\
                 }\n",
            ),
            (
                "src/test/java/other/Util.java",
                "package other;\n\
                 \n\
                 public class Util {\n\
                 \x20   public static String format(int x) { return \"\"; }\n\
                 }\n",
            ),
        ],
        &["format -> format @ src/main/java/com/example/util/Util.java"],
    );
}

/// `model.Describe()` resolves into the imported package dir; `fmt.Println`
/// never falls back to a same-dir `Println`.
#[test]
fn go_qualifier_resolves_through_package_import() {
    let files: &[(&str, &str)] = &[
        (
            "svc/app.go",
            "package svc\n\
             \n\
             import (\n\
             \x20   \"fmt\"\n\
             \x20   \"example.com/app/internal/model\"\n\
             )\n\
             \n\
             func Run() {\n\
             \x20   fmt.Println(model.Describe())\n\
             }\n",
        ),
        ("svc/print.go", "package svc\n\nfunc Println(s string) {}\n"),
        (
            "internal/model/model.go",
            "package model\n\nfunc Describe() string { return \"\" }\n",
        ),
        (
            "legacy/model/model.go",
            "package model\n\nfunc Describe() string { return \"\" }\n",
        ),
    ];
    assert_calls(files, &["Run -> Describe @ internal/model/model.go"]);
    let temp = index_project(files);
    let calls = call_edges(temp.path());
    assert!(
        !calls.contains("Run -> Println @ svc/print.go"),
        "fmt.Println must not resolve to a local Println: {calls:#?}"
    );
}

#[test]
fn python_qualifier_resolves_through_module_import() {
    assert_calls(
        &[
            (
                "agent/heartbeat.py",
                "import agent.util\n\
                 import agent.net as net\n\
                 \n\
                 \n\
                 def heartbeat():\n\
                 \x20   net.post(agent.util.load())\n",
            ),
            ("agent/util.py", "def load():\n    return ''\n"),
            ("agent/net.py", "def post(url):\n    pass\n"),
            ("frontend/net.py", "def post(url):\n    pass\n"),
        ],
        &[
            "heartbeat -> load @ agent/util.py",
            "heartbeat -> post @ agent/net.py",
        ],
    );
}

#[test]
fn rust_qualifier_resolves_through_module_and_type_imports() {
    assert_calls(
        &[
            (
                "src/b/app.rs",
                "use crate::a::shapes;\n\
                 use crate::a::model::Circle;\n\
                 \n\
                 pub fn run() {\n\
                 \x20   shapes::square(Circle::create());\n\
                 }\n",
            ),
            (
                "src/a/shapes.rs",
                "pub fn square(x: f64) -> f64 { x * x }\n",
            ),
            (
                "src/a/model.rs",
                "pub struct Circle;\n\
                 impl Circle {\n\
                 \x20   pub fn create() -> f64 { 1.0 }\n\
                 }\n",
            ),
            ("src/c/shapes.rs", "pub fn square(x: f64) -> f64 { x }\n"),
        ],
        &[
            "run -> square @ src/a/shapes.rs",
            "run -> create @ src/a/model.rs",
        ],
    );
}

#[test]
fn typescript_namespace_import_resolves_qualified_calls() {
    assert_calls(
        &[
            (
                "agent/src/heartbeat.ts",
                "import * as util from '../lib/util';\n\
                 \n\
                 export function heartbeat(): void {\n\
                 \x20 util.load();\n\
                 }\n",
            ),
            (
                "agent/lib/util.ts",
                "export function load(): string { return ''; }\n",
            ),
            (
                "frontend/lib/util.ts",
                "export function load(): string { return ''; }\n",
            ),
        ],
        &["heartbeat -> load @ agent/lib/util.ts"],
    );
}

/// Kotlin: same package across source roots, wildcard imports and imported
/// top-level functions resolve; same-named functions in other packages don't.
#[test]
fn kotlin_package_membership_and_wildcard_imports_resolve() {
    assert_calls(
        &[
            (
                "src/main/kotlin/app/a/Report.kt",
                "package app.a\n\nfun describe(x: Int): String = \"\"\n",
            ),
            (
                "src/test/kotlin/app/a/ReportTest.kt",
                "package app.a\n\nfun testDescribe() { describe(1) }\n",
            ),
            (
                "src/main/kotlin/app/b/App.kt",
                "package app.b\n\nimport app.u.*\n\nfun run() { helper() }\n",
            ),
            (
                "src/main/kotlin/app/u/Util.kt",
                "package app.u\n\nfun helper() {}\n",
            ),
            (
                "src/main/kotlin/app/c/Other.kt",
                "package app.c\n\nfun describe(x: Int): String = \"\"\nfun helper() {}\n",
            ),
        ],
        &[
            "testDescribe -> describe @ src/main/kotlin/app/a/Report.kt",
            "run -> helper @ src/main/kotlin/app/u/Util.kt",
        ],
    );
}

#[test]
fn java_package_membership_and_wildcard_imports_resolve() {
    assert_calls(
        &[
            (
                "src/main/java/app/a/Report.java",
                "package app.a;\n\
                 public class Report { public static String describe(int x) { return \"\"; } }\n",
            ),
            (
                "src/test/java/app/a/ReportTest.java",
                "package app.a;\n\
                 class ReportTest { void testDescribe() { Report.describe(1); } }\n",
            ),
            (
                "src/main/java/app/b/App.java",
                "package app.b;\n\
                 import app.u.*;\n\
                 import static app.s.Strings.pad;\n\
                 class App { void run() { Util.helper(); pad(); } }\n",
            ),
            (
                "src/main/java/app/u/Util.java",
                "package app.u;\n\
                 public class Util { public static void helper() {} }\n",
            ),
            (
                "src/main/java/app/s/Strings.java",
                "package app.s;\n\
                 public class Strings { public static void pad() {} }\n",
            ),
            (
                "src/main/java/app/c/Report.java",
                "package app.c;\n\
                 public class Report {\n\
                 \x20   public static String describe(int x) { return \"\"; }\n\
                 \x20   public static void helper() {}\n\
                 \x20   public static void pad() {}\n\
                 }\n",
            ),
        ],
        &[
            "testDescribe -> describe @ src/main/java/app/a/Report.java",
            "run -> helper @ src/main/java/app/u/Util.java",
            "run -> pad @ src/main/java/app/s/Strings.java",
        ],
    );
}

/// C#: `using App.A;` brings `App.A` types into scope; the same namespace
/// spans directories (incl. file-scoped `namespace X;`).
#[test]
fn csharp_using_and_namespace_membership_resolve() {
    assert_calls(
        &[
            (
                "A/Report.cs",
                "namespace App.A\n\
                 {\n\
                 \x20   public static class Report { public static string Describe() { return \"\"; } }\n\
                 }\n",
            ),
            (
                "B/Program.cs",
                "using System;\n\
                 using App.A;\n\
                 \n\
                 namespace App.B;\n\
                 \n\
                 public static class Program { static void Run() { Report.Describe(); Helpers.Help(); } }\n",
            ),
            (
                "Shared/Helpers.cs",
                "namespace App.B;\n\
                 \n\
                 public static class Helpers { public static void Help() {} }\n",
            ),
            (
                "C/Report.cs",
                "namespace App.C\n\
                 {\n\
                 \x20   public static class Report {\n\
                 \x20       public static string Describe() { return \"\"; }\n\
                 \x20       public static void Help() {}\n\
                 \x20   }\n\
                 }\n",
            ),
        ],
        &[
            "Run -> Describe @ A/Report.cs",
            "Run -> Help @ Shared/Helpers.cs",
        ],
    );
}

#[test]
fn php_function_imports_and_namespace_membership_resolve() {
    assert_calls(
        &[
            (
                "src/A/Report.php",
                "<?php\nnamespace App\\A;\n\nfunction describe(): string { return ''; }\n",
            ),
            (
                "src/B/App.php",
                "<?php\n\
                 namespace App\\B;\n\
                 \n\
                 use function App\\A\\describe;\n\
                 \n\
                 function run(): void { describe(); helper(); }\n",
            ),
            (
                "lib/B/Helpers.php",
                "<?php\nnamespace App\\B;\n\nfunction helper(): void {}\n",
            ),
            (
                "src/C/Other.php",
                "<?php\n\
                 namespace App\\C;\n\
                 \n\
                 function describe(): string { return ''; }\n\
                 function helper(): void {}\n",
            ),
        ],
        &[
            "run -> describe @ src/A/Report.php",
            "run -> helper @ lib/B/Helpers.php",
        ],
    );
}

#[test]
fn c_includes_resolve_to_the_matching_source_file() {
    assert_calls(
        &[
            ("a/report.h", "const char *describe(double x);\n"),
            (
                "a/report.c",
                "#include \"report.h\"\nconst char *describe(double x) { return \"\"; }\n",
            ),
            (
                "b/app.c",
                "#include \"../a/report.h\"\nvoid run(void) { describe(1.0); }\n",
            ),
            (
                "c/report.c",
                "const char *describe(double x) { return \"\"; }\n",
            ),
        ],
        &["run -> describe @ a/report.c"],
    );
}

#[test]
fn wildcard_imports_resolve_python_rust_go() {
    assert_calls(
        &[
            (
                "py/app/main.py",
                "from py.lib.util import *\n\n\ndef run():\n    helper()\n",
            ),
            ("py/lib/util.py", "def helper():\n    pass\n"),
            ("py/other/util.py", "def helper():\n    pass\n"),
            (
                "rs/src/b/app.rs",
                "use crate::a::*;\n\npub fn rrun() {\n    rhelper();\n}\n",
            ),
            ("rs/src/a/mod.rs", "pub fn rhelper() {}\n"),
            ("rs/src/c/mod.rs", "pub fn rhelper() {}\n"),
            (
                "go/app/app.go",
                "package app\n\nimport . \"example.com/m/go/lib\"\n\nfunc GRun() { GHelper() }\n",
            ),
            ("go/lib/lib.go", "package lib\n\nfunc GHelper() {}\n"),
            ("go/other/lib.go", "package other\n\nfunc GHelper() {}\n"),
        ],
        &[
            "run -> helper @ py/lib/util.py",
            "rrun -> rhelper @ rs/src/a/mod.rs",
            "GRun -> GHelper @ go/lib/lib.go",
        ],
    );
}

/// Calls resolved through Kotlin's implicit exports target the declaration,
/// not the Export node, so `callers` of the function finds them.
#[test]
fn imported_calls_target_declarations_not_exports() {
    let temp = index_project(&[
        (
            "a/Report.kt",
            "package app.a\n\nfun describe(x: Double): String = \"\"\n",
        ),
        (
            "b/App.kt",
            "package app.b\n\nimport app.a.describe\n\nfun run() { describe(2.0) }\n",
        ),
    ]);
    let calls = call_edges(temp.path());
    assert!(
        calls.contains("run -> describe @ a/Report.kt"),
        "{calls:#?}"
    );
    let non_declaration_targets = query_set(
        temp.path(),
        "SELECT s.name || ' -> ' || t.name || ' [' || t.kind || ']' FROM edges e
           JOIN nodes s ON s.id = e.source JOIN nodes t ON t.id = e.target
          WHERE e.kind = 'calls' AND t.kind NOT IN ('function', 'method')",
    );
    assert!(
        non_declaration_targets.is_empty(),
        "calls must target declarations: {non_declaration_targets:#?}"
    );
}

/// `String.format(..)` names another type: it must neither become a
/// self-edge inside `format` nor hit an unrelated same-file / same-dir
/// `format`.
#[test]
fn qualified_calls_skip_unrelated_same_file_functions() {
    let temp = index_project(&[
        (
            "k/Fmt.kt",
            "package k\n\n\
             fun format(x: Int): String = String.format(\"%d\", x)\n\
             fun pad(x: Int): String = Strings.format(x)\n",
        ),
        (
            "k/Other.kt",
            "package k\n\nfun trim(x: Int): String = Strings.format(x)\n",
        ),
        (
            "j/Fmt.java",
            "class Fmt {\n\
             \x20   String format(int x) { return String.format(\"%d\", x); }\n\
             \x20   String pad(int x) { return Strings.format(x); }\n\
             }\n",
        ),
        (
            "p/fmt.py",
            "def format(x):\n    return Strings.format(x)\n\n\
             def pad(x):\n    return str.format(x)\n",
        ),
    ]);
    let calls = call_edges(temp.path());
    let unexpected: Vec<_> = calls
        .iter()
        .filter(|call| call.contains("-> format @"))
        .collect();
    assert!(
        unexpected.is_empty(),
        "unexpected {unexpected:?} in {calls:#?}"
    );
}

/// Qualifiers naming the callee's own type (or `this` / `self`) and plain
/// variable receivers still resolve within the file.
#[test]
fn qualified_calls_resolve_to_their_own_type() {
    assert_calls(
        &[
            (
                "k/Shapes.kt",
                "package k\n\n\
                 class Circle {\n\
                 \x20   fun area(): Double = 1.0\n\
                 \x20   companion object {\n\
                 \x20       fun create(): Circle = Circle()\n\
                 \x20   }\n\
                 }\n\n\
                 fun build(): Double { val c = Circle.create(); return c.area() }\n",
            ),
            (
                "j/A.java",
                "class A {\n\
                 \x20   void run() { this.helper(); A.stat(); }\n\
                 \x20   void helper() {}\n\
                 \x20   static void stat() {}\n\
                 }\n",
            ),
            (
                "p/a.py",
                "class P:\n\
                 \x20   def go(self):\n\
                 \x20       self.assist()\n\
                 \x20       P.fixed()\n\n\
                 \x20   def assist(self):\n\
                 \x20       pass\n\n\
                 \x20   @staticmethod\n\
                 \x20   def fixed():\n\
                 \x20       pass\n",
            ),
            (
                "r/src/lib.rs",
                "pub struct Circle;\n\n\
                 impl Circle {\n\
                 \x20   pub fn make() -> Circle { Circle }\n\
                 }\n\n\
                 pub fn build() { Circle::make(); }\n",
            ),
        ],
        &[
            "build -> create @ k/Shapes.kt",
            "build -> area @ k/Shapes.kt",
            "run -> helper @ j/A.java",
            "run -> stat @ j/A.java",
            "go -> assist @ p/a.py",
            "go -> fixed @ p/a.py",
            "build -> make @ r/src/lib.rs",
        ],
    );
}

/// `caller -> callee_qualified_name conf` for every stored calls edge.
fn call_targets(project_path: &Path) -> BTreeSet<String> {
    query_set(
        project_path,
        "SELECT s.name || ' -> ' || replace(t.qualified_name, '\\', '/') || ' '
                || printf('%.2f', e.confidence)
           FROM edges e
           JOIN nodes s ON s.id = e.source JOIN nodes t ON t.id = e.target
          WHERE e.kind = 'calls'",
    )
}

fn targets_of<'a>(calls: &'a BTreeSet<String>, caller: &str) -> Vec<&'a str> {
    let prefix = format!("{caller} -> ");
    calls
        .iter()
        .filter(|call| call.starts_with(&prefix))
        .map(String::as_str)
        .collect()
}

/// Same-named methods in one file: the caller's own class wins for
/// unqualified and `this` calls, the named type for `Type.m()`.
#[test]
fn same_named_methods_resolve_by_class() {
    let temp = index_project(&[
        (
            "j/Shapes.java",
            "class Circle {\n\
             \x20   double area() { return scale(); }\n\
             \x20   double perimeter() { return this.scale(); }\n\
             \x20   double scale() { return 1.0; }\n\
             \x20   static Circle create() { return null; }\n\
             }\n\
             class Square {\n\
             \x20   double scale() { return 2.0; }\n\
             \x20   static Square create() { return null; }\n\
             \x20   static void build() { Circle.create(); }\n\
             }\n",
        ),
        (
            "k/Shapes.kt",
            "class Circle {\n\
             \x20   fun area(): Double = scale()\n\
             \x20   fun scale(): Double = 1.0\n\
             \x20   companion object {\n\
             \x20       fun create(): Circle = Circle()\n\
             \x20   }\n\
             }\n\
             class Square {\n\
             \x20   fun scale(): Double = 2.0\n\
             \x20   companion object {\n\
             \x20       fun create(): Square = Square()\n\
             \x20   }\n\
             }\n\
             fun build() { Square.create() }\n",
        ),
    ]);
    let calls = call_targets(temp.path());
    for (caller, expected) in [
        (
            "area",
            vec![
                "area -> j/Shapes.java::Circle::scale 0.50",
                "area -> k/Shapes.kt::Circle::scale 0.50",
            ],
        ),
        (
            "perimeter",
            vec!["perimeter -> j/Shapes.java::Circle::scale 0.50"],
        ),
        (
            "build",
            vec![
                "build -> j/Shapes.java::Circle::create 1.00",
                "build -> k/Shapes.kt::Square::Companion::create 1.00",
            ],
        ),
    ] {
        assert_eq!(targets_of(&calls, caller), expected, "{calls:#?}");
    }
}

/// Overloads of one method all get an edge; calls on receivers of unknown
/// type with a few same-file candidates get low-confidence edges to each,
/// but never to candidates outside the import / file / dir / package tiers.
#[test]
fn ambiguous_calls_link_every_candidate_with_lower_confidence() {
    let temp = index_project(&[
        (
            "src/main/java/app/util/Util.java",
            "package app.util;\n\n\
             public class Util {\n\
             \x20   public static String format(int x) { return \"\"; }\n\
             \x20   public static String format(String s) { return s; }\n\
             }\n",
        ),
        (
            "src/main/java/app/App.java",
            "package app;\n\n\
             import app.util.Util;\n\n\
             class App {\n\
             \x20   String render() { return Util.format(1); }\n\
             }\n",
        ),
        (
            "py/shapes.py",
            "class Circle:\n    def area(self):\n        return 1\n\n\
             class Square:\n    def area(self):\n        return 2\n\n\
             def total(shape):\n    return shape.area()\n",
        ),
        (
            "other/sizes.py",
            "class Disc:\n    def area(self):\n        return 3\n",
        ),
        (
            "third/report.py",
            "def summary(shape):\n    return shape.area()\n",
        ),
    ]);
    let calls = call_targets(temp.path());
    let render = targets_of(&calls, "render");
    assert_eq!(
        render.len(),
        1,
        "both overloads share a qualified name: {calls:#?}"
    );
    let overloads = query_set(
        temp.path(),
        "SELECT t.id || ' ' || printf('%.2f', e.confidence) FROM edges e
           JOIN nodes s ON s.id = e.source JOIN nodes t ON t.id = e.target
          WHERE e.kind = 'calls' AND s.name = 'render'",
    );
    assert_eq!(overloads.len(), 2, "{overloads:#?}");
    assert!(
        overloads.iter().all(|o| o.ends_with(" 0.50")),
        "{overloads:#?}"
    );
    assert_eq!(
        targets_of(&calls, "total"),
        vec![
            "total -> py/shapes.py::Circle::area 0.30",
            "total -> py/shapes.py::Square::area 0.30",
        ],
        "{calls:#?}"
    );
    // Ambiguity never widens the search: no import, no same dir → no edge.
    assert!(targets_of(&calls, "summary").is_empty(), "{calls:#?}");
}

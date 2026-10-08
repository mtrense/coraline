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
    let crossing = cross_root_calls(temp.path());
    assert!(
        crossing.is_empty(),
        "unexpected cross-root calls {crossing:?}"
    );
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

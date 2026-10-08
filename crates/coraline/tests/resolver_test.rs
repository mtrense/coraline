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

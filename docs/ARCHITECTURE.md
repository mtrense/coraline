# Coraline Architecture

Coraline is a local-first code intelligence system that builds a semantic knowledge graph from any codebase. It uses tree-sitter for deterministic AST-based parsing, SQLite for storage and full-text search, and exposes its capabilities via both a CLI and an MCP server.

---

## High-Level Overview

![Coraline high-level architecture](assets/architecture-overview.png)

Diagram source: `docs/diagrams/high-level-overview.mmd`

---

## Source Layout

```
crates/coraline/src/
├── bin/coraline.rs     # CLI entry point (clap)
├── lib.rs              # Public API surface
├── types.rs            # NodeKind, EdgeKind, all shared types
├── db.rs               # SQLite layer + schema + FTS
├── extraction.rs       # Tree-sitter parsing + indexing pipeline
├── graph.rs            # Graph traversal and subgraph queries
├── clustering.rs       # Louvain community detection + process tracing (Phase 5.1)
├── resolution/         # Cross-file reference resolution
│   ├── mod.rs          # Core resolver + framework fallback
│   ├── import_path.rs  # Import module paths → project files
│   ├── receiver.rs     # Which declarations a call qualifier / receiver admits
│   ├── swift_module.rs # Swift modules from Package.swift / *.xcodeproj
│   └── frameworks/     # Language/framework-specific resolvers
│       ├── mod.rs      # FrameworkResolver trait + registry
│       ├── rust.rs     # crate::, super::, self:: resolution
│       ├── react.rs    # ./Foo imports, @/ aliases, components
│       ├── blazor.rs   # .razor file discovery, .NET types
│       └── laravel.rs  # PSR-4, blade views, facades
├── context.rs          # Context builder (Markdown/JSON output)
├── vectors.rs          # Vector storage + cosine similarity + ONNX model management
├── vec_ext.rs          # Optional sqlite-vec integration (Phase 5.3, `--features vec-ext`)
├── memory.rs           # Project memory CRUD
├── config.rs           # TOML + JSON configuration loading
├── sync.rs             # Incremental sync + git hook management
├── logging.rs          # Structured logging (tracing)
├── doctor.rs           # Self-healing probes (config, db, model, hooks, embed coverage)
├── security.rs         # Path/permission guards for MCP file tools
├── audit.rs            # Lightweight structured event log for tool invocations
├── update.rs           # Self-update helper (cargo install --force flow)
├── mcp.rs              # MCP server (JSON-RPC over stdio)
├── utils.rs            # Shared utilities
├── db/                 # Schema migrations + table-level helpers
└── tools/
    ├── mod.rs          # Tool trait + ToolRegistry
    ├── graph_tools.rs  # search, callers, callees, impact, find_symbol, ...
    ├── context_tools.rs# coraline_context
    ├── file_tools.rs   # read_file, list_dir, status, config
    └── memory_tools.rs # write/read/list/delete/edit memory
```

---

## Data Model

### Nodes

Every extracted symbol is a `Node`:

```rust
pub struct Node {
    pub id: String,              // SHA-based deterministic ID
    pub kind: NodeKind,          // function, class, method, struct, ...
    pub name: String,            // unqualified symbol name
    pub qualified_name: Option<String>,
    pub file_path: String,       // absolute path
    pub language: String,
    pub start_line: i64,
    pub end_line: i64,
    pub start_column: i64,
    pub end_column: i64,
    pub signature: Option<String>,
    pub docstring: Option<String>,
    pub visibility: Option<String>,
    pub is_exported: bool,
    pub is_async: bool,
    pub is_static: bool,
    pub is_abstract: bool,
    pub decorators: Vec<String>,
    pub type_parameters: Vec<String>,
    pub cluster_id: Option<i64>, // Phase 5.1 — Louvain community id; None for unclustered nodes
}
```

**NodeKind values:** `file`, `module`, `class`, `struct`, `interface`, `trait`, `protocol`, `function`, `method`, `property`, `field`, `variable`, `constant`, `enum`, `enum_member`, `type_alias`, `namespace`, `parameter`, `import`, `export`, `route`, `component`

### Edges

Relationships between nodes are `Edge` records:

```rust
pub struct Edge {
    pub id: String,
    pub source: String,          // source node ID
    pub target: String,          // target node ID
    pub kind: EdgeKind,
    pub line: Option<i64>,       // line where the relationship occurs
    pub metadata: Option<String>,
    pub confidence: f32,         // Phase 5.2 — in [0.0, 1.0], see table below
    pub process_id: Option<i64>, // Phase 5.1 — execution-flow trace id; None for edges not in any process trace
}
```

**EdgeKind values:** `contains`, `calls`, `imports`, `exports`, `extends`, `implements`, `references`, `type_of`, `returns`, `instantiates`, `overrides`, `decorates`

Extraction emits `contains`, `calls`, `imports`, `exports`, `extends`, `implements` and `instantiates`; the other kinds are defined but not emitted yet. `extends` / `implements` come from supertype clauses (a class / struct naming an interface, protocol or trait implements it, otherwise extends; Rust `impl Trait for Type` implements). `instantiates` comes from explicit constructions (`new Foo()`, `Foo.new`, `Foo{…}`) and from calls whose callee resolves to a class / struct (`Circle(2.0)`); it points at the type, never at a constructor method. Type references are resolved with the same scoping rules as calls (same file, same dir, imports, packages / namespaces / modules; no project-wide name match).

Every `Edge` carries a `confidence: f32` in `[0.0, 1.0]`:

- `1.0` — direct AST-extracted (caller wrote the symbol syntactically; no resolution was needed)
- `0.95` — strongly-typed Rust path (`crate::` / `super::` / `self::`)
- `0.5` — generic name match / framework fallback / heuristic ranker (also each overload when a call matches several overloads of one method)
- `0.3` — one of up to three equally ranked call targets of different types (`shape.area()` with `Circle.area` and `Square.area` in scope); the resolver can't tell which without type information

`coraline_callers` / `coraline_callees` / `coraline_find_references` accept a `min_confidence` parameter that filters edges below a caller-supplied threshold. Default `0.0` includes every edge; a typical working value is `0.8` to suppress generic matches.

---

## Indexing Pipeline

1. **Scan** — Glob the project tree using `include_patterns`/`exclude_patterns`.
2. **Parse** — For each file, spawn the appropriate tree-sitter grammar and walk the AST.
3. **Extract** — Emit `Node` and `Edge` records from the AST visitor. Every `Edge` carries a `confidence` field (see above). A call's source is the innermost enclosing function or method; calls outside any (property / field initializers, Kotlin `init {}` blocks and getters, class-field arrow functions, top-level script code) are attributed to the innermost enclosing type, else to the file node.
4. **Store** — Upsert nodes and edges into SQLite. A file content hash prevents re-parsing unchanged files.
5. **Resolve** — Walk `unresolved` reference edges, attempt name-based resolution in the DB; fall back to framework-specific resolvers for zero-candidate references. Resolved edges are stamped with their `confidence`.
6. **Cluster** _(Phase 5.1)_ — Run Louvain community detection over the call graph and write `nodes.cluster_id`. Detects "module-like" communities (groups of nodes that call each other more than they call outside the group).
7. **Trace processes** _(Phase 5.1)_ — For each exported `Function`/`Method` with no incoming `calls` edge (an "entry point"), DFS forward through the call graph with cycle protection and a depth cap, writing `edges.process_id`. Every edge that participates in some process trace gets a `process_id`.

Steps 6 and 7 run as part of every `coraline index` and `coraline sync` after extraction succeeds. The columns are nullable — `cluster_id IS NULL` and `process_id IS NULL` are normal for nodes/edges that don't participate in any cluster or process trace.

### Incremental Sync

`coraline sync` (and the git post-commit hook) uses `git diff --name-only HEAD~1` to find changed files, then re-parses only those files. Deleted files have their nodes pruned from the graph.

---

## Reference Resolution

Resolution happens in two passes:

1. **Name-based**: The `resolution::resolve_unresolved` function looks up reference names in the DB and ranks the candidates. A call's qualifier first rules out candidates it can't refer to (`resolution/receiver.rs`): `String.format(..)` never resolves to a same-file `format` (nor to the caller itself), `Circle.create()` only to members of `Circle` (incl. its companion), `this.m()` / `self.m()` only to methods. Tiers:
   1. Declarations behind the import that binds the name (`import { describe }`, `use crate::a::describe`, Kotlin `import app.a.describe`; Export nodes are followed to the declaration). Import module paths are mapped to project files per language (`resolution/import_path.rs`: dotted / `::` / `\` paths, relative `./`, `.x`, `crate::`, `super::`, quoted includes) or to the packages / namespaces files declare.
   2. If the call's qualifier names an import (`Report.describe()`, Go `model.Describe()`, `util.load()`), only declarations in that import's module, otherwise nothing.
   3. Same file, then same directory.
   4. Names in scope without naming the callee: wildcard imports (`app.a.*`, `from x import *`, `use a::*`, Go `.`), C/C++ includes, Ruby `require` / `require_relative`, C# `using` namespaces, Swift `import <Target>` (SPM target), and the caller's own package / namespace / Swift module. Swift modules come from build manifests only (`resolution/swift_module.rs`): each `Sources/<Target>/` / `Tests/<Target>/` of a `Package.swift` package, or everything below a directory containing a `*.xcodeproj`; own-module declarations shadow imported ones.
   For calls there is no further fallback: same-named functions in unrelated directories are never linked by name alone (upstream #43).
   If the best tier holds several candidates, calls are narrowed by receiver (`Type.m()` → members of `Type`; unqualified and `this.m()` → the caller's own type). Overloads of one method all get an edge; up to three candidates of different types get an edge each with confidence `0.3`; more stay unresolved. Other reference kinds and framework fallbacks need exactly one candidate.

2. **Framework fallback**: When no candidates score above threshold, `framework_fallback` is called. The registered `FrameworkResolver` implementations detect the active framework (by checking for `Cargo.toml`, `package.json`, `artisan`, `.csproj`, etc.) and return candidate file paths. Nodes from those files are then loaded and filtered by the referenced symbol name.

Current framework resolvers:

- **RustResolver** — `crate::`, `super::`, `self::` qualified paths → `.rs` file mapping
- **ReactResolver** — `./Foo` relative imports, `@/` path aliases, PascalCase component search
- **BlazorResolver** — PascalCase component → `.razor` file, dot-qualified .NET types
- **LaravelResolver** — PSR-4 FQN → PHP file, dot-notation views → blade templates

---

## Tool Architecture

All MCP tools implement the `Tool` trait:

```rust
pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn input_schema(&self) -> Value;
    fn execute(&self, params: Value) -> ToolResult;
}
```

Tools are registered in a `ToolRegistry`, which:

- Dispatches `tools/call` MCP requests by name
- Automatically generates `tools/list` responses from registered metadata
- Can be used outside MCP (CLI, library API, tests)

---

## Database Schema

The SQLite database (`.coraline/coraline.db`) has these tables:

| Table             | Purpose                                                                                                                                                      |
| ----------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `nodes`           | All indexed symbols with full metadata, plus a nullable `cluster_id` (Phase 5.1, Louvain community id)                                                       |
| `edges`           | Directed relationships between nodes, plus a `confidence REAL` (Phase 5.2, in `[0.0, 1.0]`) and a nullable `process_id` (Phase 5.1, execution-flow trace id) |
| `nodes_fts`       | FTS5 virtual table for fast name search                                                                                                                      |
| `vectors`         | Optional embeddings storage. `embedding BLOB` (v1) or vec0 `vec0` virtual table (`--features vec-ext`)                                                       |
| `schema_versions` | Additive-migration bookkeeping. Every additive `ALTER TABLE` bumps a version row here.                                                                       |

A `files` table tracks content hashes for incremental sync. An `unresolved_refs` table holds references that couldn't be resolved during extraction; every resolution pass pages through all of them by id, and refs that resolve are deleted. Before a file's nodes are deleted (re-index or removal), resolved edges from other files into it are queued again as unresolved refs (the call-site name and qualifier are kept in the edge's `metadata`), so they re-link to the re-indexed target.

### Additive migrations

DB schema changes after v1 are additive: each new column is gated by a `PRAGMA table_info(<table>)` check in `db::apply_incremental_migrations` and stamped with a row in `schema_versions`. Existing DBs migrate in place on next `coraline init` / `coraline sync` without backfill. The migration order is:

- v2 — `edges.confidence REAL NOT NULL DEFAULT 1.0` (Phase 5.2)
- v3 — `nodes.cluster_id INTEGER`, `edges.process_id INTEGER` (Phase 5.1)
- v4 — vec0 embedding tables (`--features vec-ext`, created by `vec_ext`)
- v5 — `unresolved_refs.qualifier TEXT`: call receiver / qualifier (`Report` in `Report.describe()`) for import-backed resolution

All migrations are idempotent — the `column_exists` guard makes re-running safe.

---

## MCP Protocol

The MCP server (`coraline serve --mcp`) communicates over `stdin`/`stdout` using JSON-RPC 2.0, conforming to the [Model Context Protocol specification](https://modelcontextprotocol.io/).

Supported MCP methods:

- `initialize` / `notifications/initialized`
- `tools/list` — returns tool descriptors with cursor pagination (`cursor` / `nextCursor`)
- `tools/call` — dispatches to `ToolRegistry`
- `ping`

The server is single-threaded and synchronous; each request is fully processed before the next is read.

---

## Logging

Structured logs use the `tracing` crate:

- **Console**: stderr at the level set by `CORALINE_LOG` (default: `info`)
- **File**: `.coraline/logs/coraline.log` with daily rotation (kept 7 days)

```bash
CORALINE_LOG=debug coraline index      # verbose
CORALINE_LOG=warn coraline serve --mcp # quiet
```

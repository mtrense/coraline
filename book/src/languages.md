# Supported Languages

Coraline uses [tree-sitter](https://tree-sitter.github.io/) for parsing, enabling support for 17 languages with AST-level precision.

## Language Support Table

| Language | Status | tree-sitter grammar | Framework Support |
|---|---|---|---|
| **C** | ✅ Full | `tree-sitter-c` 0.23 | - |
| **C++** | ✅ Full | `tree-sitter-cpp` 0.23 | - |
| **C#** | ✅ Full | `tree-sitter-c-sharp` 0.23 | .NET |
| **Blazor** | ⚠️ File-level only (no symbols) | - | Blazor components |
| **Go** | ✅ Full | `tree-sitter-go` 0.23 | - |
| **Java** | ✅ Full | `tree-sitter-java` 0.23 | Spring |
| **JavaScript** | ✅ Full | `tree-sitter-javascript` 0.25.0 | Node.js |
| **JSX** | ✅ Full | `tree-sitter-javascript` 0.25.0 | React |
| **Kotlin** | ✅ Full | `tree-sitter-kotlin-ng` 1.1.0 | Android |
| **Markdown** | ✅ Full | `tree-sitter-markdown-fork` 0.7.3 | - |
| **PHP** | ✅ Full | `tree-sitter-php` 0.24.2 | Laravel |
| **Python** | ✅ Full | `tree-sitter-python` 0.23 | Django, Flask |
| **Ruby** | ✅ Full | `tree-sitter-ruby` 0.23 | Rails |
| **Rust** | ✅ Full | `tree-sitter-rust` 0.24.0 | - |
| **Swift** | ✅ Full | `tree-sitter-swift` 0.7.1 | iOS/macOS |
| **TypeScript** | ✅ Full | `tree-sitter-typescript` 0.23.1 | Node.js |
| **TSX** | ✅ Full | `tree-sitter-typescript` 0.23.1 | React |

## Language Detection

Coraline detects languages by file extension:

| Extension(s) | Language |
|---|---|
| `.rs` | Rust |
| `.ts` | TypeScript |
| `.tsx` | TSX (TypeScript + JSX) |
| `.js` | JavaScript |
| `.jsx` | JSX (JavaScript + JSX) |
| `.py` | Python |
| `.go` | Go |
| `.java` | Java |
| `.c`, `.h` | C |
| `.cpp`, `.cc`, `.cxx`, `.hpp` | C++ |
| `.cs` | C# |
| `.php` | PHP |
| `.rb` | Ruby |
| `.swift` | Swift |
| `.kt`, `.kts` | Kotlin |
| `.razor` | Blazor |
| `.md`, `.markdown` | Markdown |

## Node Types by Language

Different languages produce different node kinds:

### Object-Oriented (Java, C#, Swift, Kotlin)
- `class`
- `interface`
- `method`
- `property`
- `field`
- `enum`
- `enum_member`

### Systems (Rust, C, C++)
- `struct`
- `function`
- `trait` (Rust)
- `enum`
- `type_alias`
- `module` (Rust)

### Scripting (Python, Ruby, PHP)
- `function`
- `class`
- `method`
- `variable`
- `constant`

### Web Frontend (JavaScript, TypeScript, TSX, JSX)
- `function`
- `class`
- `method`
- `component` (React/JSX/TSX)
- `interface` (TypeScript)
- `type_alias` (TypeScript)

### Markup (Markdown)
- `module` (file-level)

## Inheritance and Instantiation Edges

| Language | `extends` / `implements` from | `instantiates` from |
|---|---|---|
| Kotlin | delegation specifiers (`: Base(), I`) | calls resolving to a class (`Circle(2.0)`) |
| Java | `extends`, `implements`, interface `extends` | `new Foo()` |
| C# | base list (`: Base, IFoo`) | `new Foo()` |
| TypeScript / JavaScript | `extends`, `implements`, interface `extends` | `new Foo()` |
| Swift | inheritance specifiers (`: Base, P`) | `Foo<T>()`, calls resolving to a class / struct |
| Python | class bases (`class C(Base)`) | calls resolving to a class |
| Ruby | `class C < Base` | `Foo.new` |
| PHP | `extends`, `implements` | `new Foo()` |
| C++ | base classes (`: public Base`) | `new Foo()`, calls resolving to a class |
| Rust | `impl Trait for Type` (type declared in the same file) | struct expressions (`Foo { … }`) |
| Go | - (interfaces are implicit) | composite literals (`pkg.Foo{…}`) |

Where the syntax doesn't say which (Kotlin, C#, Swift, C++), a class / struct naming an interface, protocol or trait gets `implements`, everything else `extends`.

## Framework-Specific Features

### Rust
- Crate-relative imports (`crate::`, `super::`, `self::`)
- Trait implementation tracking
- Macro detection
- Module tree resolution

### React (JavaScript/TypeScript/JSX/TSX)
- Component detection (PascalCase)
- Hook tracking (`use*` functions)
- Relative imports (`./`, `../`)
- Path aliases (`@/`)

### Blazor (C# + Razor)
- Razor component detection (`.razor` files)
- Component parameter tracking
- .NET type resolution
- Dependency injection detection

### Laravel (PHP)
- PSR-4 autoloading resolution
- Blade template tracking
- Facade detection
- Dot-notation view paths

### Python
- Module imports (`from ... import`)
- Package structure
- Class/function decorators
- Django/Flask detection (future)

### Others
- **Swift/Kotlin**: Protocol/interface conformance
- **Java**: Spring annotations, package resolution

## Cross-Language Support

Coraline can index **polyglot projects** with multiple languages:

```
project/
├── backend/           (Rust)
├── frontend/          (TypeScript + React)
├── mobile/            (Swift, Kotlin)
└── scripts/           (Python)
```

All languages are indexed into a unified graph. Cross-language references (e.g., TypeScript calling Rust WASM) are tracked as `unresolved` edges unless framework resolvers are available.

## Language-Specific Configuration

Customize indexing per language via `config.toml`:

```toml
[indexing]
include_patterns = [
  "src/**/*.rs",           # Rust source
  "lib/**/*.ts",           # TypeScript libraries
  "app/**/*.tsx",          # React components
  "**/*.py",               # Python anywhere
]
exclude_patterns = [
  "**/target/**",          # Rust builds
  "**/node_modules/**",    # JS/TS deps
  "**/__pycache__/**",     # Python cache
  "**/dist/**",            # Build outputs
]
```

## Adding New Languages

Coraline can be extended with new tree-sitter grammars:

1. Add the grammar to `Cargo.toml`:
   ```toml
   tree-sitter-newlang = "0.1.0"
   ```

2. Register in `extraction.rs`:
   ```rust
   ".newlang" => tree_sitter_newlang::language(),
   ```

3. Add extraction patterns for node/edge types

4. (Optional) Add framework resolver in `resolution/frameworks/`

See [Development Guide](./development.md) for contribution instructions.

## Language Limitations

### Markdown
- Only headings and code blocks are indexed
- No inline link tracking

### Future Enhancements
- SQL query extraction (embedded SQL strings)
- HTML/CSS parsing (currently ignored)
- GraphQL schema tracking
- Protocol Buffers support

## Performance by Language

Parse speed varies by grammar complexity:

| Language Group | Relative Speed | Notes |
|---|---|---|
| Rust, Go, C | Fast | Simple, deterministic grammars |
| TypeScript, Python | Medium | More complex syntax rules |
| C++ | Slower | High grammar complexity |
| Markdown | Very Fast | Simple structure |

Actual impact on indexing is minimal (<5% variation) for most projects.

## Testing Language Support

Verify language support for your project:

```bash
# Index and check stats
coraline init -i
coraline stats --json

# Check language breakdown
jq '.files_by_language' .coraline/stats.json
```

Output:
```json
{
  "rust": 42,
  "typescript": 31,
  "python": 12,
  "markdown": 5
}
```

Search for language-specific symbols:

```bash
# Rust
coraline query "impl" --kind trait

# TypeScript
coraline query "interface" --kind interface

# Python
coraline query "def" --kind function
```

## Requesting Language Support

To request a new language:

1. Check if a tree-sitter grammar exists at [tree-sitter.github.io](https://tree-sitter.github.io/)
2. Open an issue at [github.com/greysquirr3l/coraline/issues](https://github.com/greysquirr3l/coraline/issues)
3. Include:
   - Language name and typical file extensions
   - Link to tree-sitter grammar
   - Example files for testing
   - (Optional) Framework-specific features needed

## Next Steps

- [Quick Start Guide](./quick-start.md) - Start indexing your project
- [Configuration Guide](./configuration.md) - Customize language filtering
- [Architecture](./architecture.md) - How language parsing works
- [Development Guide](./development.md) - Contribute new languages

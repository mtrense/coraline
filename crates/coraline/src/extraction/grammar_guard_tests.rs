//! Grammar guard: every node kind and field name the extractor relies on must
//! exist in the tree-sitter grammar of the language it is used for.
//!
//! A misspelt or outdated kind/field name fails silently at runtime (the node
//! is simply never matched), which is how whole edge categories went missing
//! for Kotlin, Java, Swift, Go, ... This test turns such mistakes into a test
//! failure.
//!
//! Checks are grammar-global: `field_id_for_name` only tells whether a field
//! exists *somewhere* in the grammar, not on a specific node kind.
//!
//! `known_bad` is a temporary allowlist of names that are currently wrong. The
//! test asserts the detected set equals the allowlist exactly, so:
//! - fixing a mapping requires removing its entry from the allowlist, and
//! - introducing a new bad name fails the test.
//!
//! Goal: shrink every allowlist entry to empty, then delete `known_bad`.

use std::collections::BTreeSet;

use super::{
    call_expression_kinds, call_name_fields, import_path_field, language_to_parser,
    node_kind_mappings,
};
use crate::config::is_language_supported;
use crate::types::{Language, NodeKind};

const ALL_LANGUAGES: &[Language] = &[
    Language::TypeScript,
    Language::JavaScript,
    Language::Tsx,
    Language::Jsx,
    Language::Python,
    Language::Go,
    Language::Rust,
    Language::Java,
    Language::C,
    Language::Cpp,
    Language::CSharp,
    Language::Php,
    Language::Ruby,
    Language::Swift,
    Language::Kotlin,
    Language::Liquid,
    Language::Blazor,
    Language::Bash,
    Language::Dart,
    Language::Elixir,
    Language::Elm,
    Language::Erlang,
    Language::Fortran,
    Language::Groovy,
    Language::Haskell,
    Language::Julia,
    Language::Lua,
    Language::Markdown,
    Language::Matlab,
    Language::Nix,
    Language::Perl,
    Language::Powershell,
    Language::R,
    Language::Scala,
    Language::Toml,
    Language::Yaml,
    Language::Zig,
    Language::Unknown,
];

/// Kinds and fields referenced by language-specific helper code (visibility,
/// import/export symbol collection, module names) rather than by the mapping
/// tables. Keep in sync with the string literals in `extraction.rs`.
fn helper_names(language: Language) -> (&'static [&'static str], &'static [&'static str]) {
    match language {
        // read_declaration_visibility, rust_use_symbols, module_name
        Language::Rust => (
            &[
                "visibility_modifier",
                "use_list",
                "scoped_use_list",
                "use_wildcard",
                "use_as_clause",
            ],
            &["argument", "list", "alias", "path", "name"],
        ),
        Language::JavaScript | Language::Jsx | Language::TypeScript | Language::Tsx => (
            &[
                "export_statement",
                "import_clause",
                "namespace_import",
                "named_imports",
                "import_specifier",
                "export_specifier",
                "function_declaration",
                "class_declaration",
                "interface_declaration",
                "type_alias_declaration",
                "enum_declaration",
                "variable_declarator",
            ],
            &["name", "alias", "source"],
        ),
        Language::Python => (&[], &["name", "alias"]),
        // go_import_symbols, go_callee
        Language::Go => (
            &[
                "import_spec",
                "import_spec_list",
                "identifier",
                "field_identifier",
                "type_identifier",
                "selector_expression",
                "qualified_type",
                "index_expression",
                "type_instantiation_expression",
                "parenthesized_expression",
            ],
            &["path", "name", "field", "operand", "type"],
        ),
        // read_declaration_visibility
        Language::Java => (&["modifiers"], &[]),
        // read_declaration_visibility, csharp_using_symbols
        Language::CSharp => (
            &[
                "modifier",
                "identifier",
                "qualified_name",
                "generic_name",
                "alias_qualified_name",
            ],
            &["name"],
        ),
        // kotlin_callee, import_module_path, kotlin_node_name
        Language::Kotlin => (
            &[
                "identifier",
                "navigation_expression",
                "qualified_identifier",
                "variable_declaration",
                // kotlin_class_kind
                "modifiers",
                "class_modifier",
                // kotlin_export_symbol
                "class_declaration",
                "object_declaration",
                "function_declaration",
                "property_declaration",
                "type_alias",
                "package_header",
                "visibility_modifier",
            ],
            // kotlin_node_name: `type_alias` name, `companion_object` name
            &["type", "name"],
        ),
        // swift_callee
        Language::Swift => (
            &[
                "constructor_expression",
                "user_type",
                "type_identifier",
                "simple_identifier",
                "navigation_expression",
                // swift_class_kind
                "class_declaration",
                "deinit_declaration",
            ],
            &["constructed_type", "suffix", "declaration_kind"],
        ),
        // c_function_name, c_callee
        Language::C => (
            &[
                "function_definition",
                "identifier",
                "field_identifier",
                "parenthesized_declarator",
                "attributed_declarator",
                "ms_call_modifier",
                "attribute_declaration",
                "field_expression",
            ],
            &["declarator", "function", "field"],
        ),
        // c_function_name, cpp_is_method, c_callee
        Language::Cpp => (
            &[
                "function_definition",
                "identifier",
                "field_identifier",
                "destructor_name",
                "operator_name",
                "qualified_identifier",
                "template_function",
                "template_method",
                "parenthesized_declarator",
                "reference_declarator",
                "attributed_declarator",
                "ms_call_modifier",
                "attribute_declaration",
                "field_declaration_list",
                "field_expression",
            ],
            &["declarator", "name", "function", "field"],
        ),
        // php_callee, php_import_symbols
        Language::Php => (
            &[
                "object_creation_expression",
                "name",
                "qualified_name",
                "namespace_name",
                "namespace_use_clause",
            ],
            &["body", "alias"],
        ),
        // ruby_callee
        Language::Ruby => (
            &["constant", "scope_resolution"],
            &["method", "receiver", "name"],
        ),
        _ => (&[], &[]),
    }
}

/// Languages whose mapping tables are shared. A name counts as valid if it
/// exists in any grammar of the group (e.g. `interface_declaration` is
/// TypeScript-only but harmless in the shared JS/TS table).
fn grammar_group(language: Language) -> Vec<Language> {
    match language {
        Language::JavaScript | Language::Jsx | Language::TypeScript | Language::Tsx => {
            vec![Language::JavaScript, Language::TypeScript, Language::Tsx]
        }
        other => vec![other],
    }
}

/// Temporary allowlist of names known not to exist in the grammar.
/// Entries are `"kind:<name>"` or `"field:<name>"`. Remove entries as the
/// corresponding mappings get fixed (see plan §2/§3).
fn known_bad(language: Language) -> &'static [&'static str] {
    match language {
        Language::JavaScript | Language::Jsx | Language::TypeScript | Language::Tsx => {
            &["field:callee", "kind:export_declaration"]
        }
        _ => &[],
    }
}

fn bad_names(language: Language) -> BTreeSet<String> {
    let grammars: Vec<tree_sitter::Language> = grammar_group(language)
        .into_iter()
        .filter_map(language_to_parser)
        .collect();
    let kind_exists = |kind: &str| grammars.iter().any(|g| g.id_for_node_kind(kind, true) != 0);
    let field_exists = |field: &str| {
        grammars
            .iter()
            .any(|g| g.field_id_for_name(field).is_some())
    };

    let mappings = node_kind_mappings(language);
    let (helper_kinds, helper_fields) = helper_names(language);

    let kinds = mappings
        .iter()
        .map(|(kind, _, _)| *kind)
        .chain(call_expression_kinds(language).iter().copied())
        .chain(helper_kinds.iter().copied());

    let has_imports = mappings
        .iter()
        .any(|(_, node_kind, _)| *node_kind == NodeKind::Import);
    let import_field = has_imports.then(|| import_path_field(language)).flatten();
    let fields = call_name_fields(language)
        .iter()
        .copied()
        .chain(import_field)
        .chain(helper_fields.iter().copied());

    kinds
        .filter(|kind| !kind_exists(kind))
        .map(|kind| format!("kind:{kind}"))
        .chain(
            fields
                .filter(|field| !field_exists(field))
                .map(|field| format!("field:{field}")),
        )
        .collect()
}

#[test]
fn extraction_names_exist_in_grammars() {
    let mut failures = Vec::new();

    for &language in ALL_LANGUAGES {
        if !is_language_supported(&language) || language_to_parser(language).is_none() {
            continue;
        }

        let actual = bad_names(language);
        let allowed: BTreeSet<String> = known_bad(language)
            .iter()
            .map(|s| (*s).to_string())
            .collect();

        let new_bad: Vec<_> = actual.difference(&allowed).collect();
        let fixed: Vec<_> = allowed.difference(&actual).collect();
        if !new_bad.is_empty() {
            failures.push(format!(
                "{language:?}: names missing from grammar (fix the mapping): {new_bad:?}"
            ));
        }
        if !fixed.is_empty() {
            failures.push(format!(
                "{language:?}: allowlisted names now valid or unused (remove from known_bad): {fixed:?}"
            ));
        }
    }

    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// Unsupported languages are never parsed (`is_language_supported` gates
/// indexing), so any parser or mapping for them is dead, unchecked code.
/// To add a language, enable it in `is_language_supported` together with
/// its mappings so the guard above covers them.
#[test]
fn unsupported_languages_have_no_extraction_code() {
    let offenders: Vec<Language> = ALL_LANGUAGES
        .iter()
        .copied()
        .filter(|language| !is_language_supported(language))
        .filter(|&language| {
            language_to_parser(language).is_some()
                || !node_kind_mappings(language).is_empty()
                || !call_expression_kinds(language).is_empty()
                || !call_name_fields(language).is_empty()
        })
        .collect();

    assert!(
        offenders.is_empty(),
        "unsupported languages with parser/mappings: {offenders:?}"
    );
}

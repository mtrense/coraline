//! Grammar guard: every node kind and field name the extractor relies on must
//! exist in the tree-sitter grammar of the language it is used for.
//!
//! A misspelt or outdated kind/field name fails silently at runtime (the node
//! is simply never matched), which is how whole edge categories went missing
//! for Kotlin, Java, Swift, Go, ... This test turns such mistakes into a test
//! failure.
//!
//! Checks are grammar-global: `field_id_for_name` only tells whether a field
//! exists *somewhere* in the grammar, not on a specific node kind, so per-node
//! field mistakes need fixture tests (`tests/<lang>_extraction_test.rs`).

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
                // js_function_value_name
                "identifier",
                // type_refs
                "abstract_class_declaration",
                "class_heritage",
                "extends_clause",
                "implements_clause",
                "extends_type_clause",
            ],
            &["name", "alias", "source", "value", "type"],
        ),
        // python_import_symbols
        Language::Python => (
            // type_refs
            &[
                "import_statement",
                "aliased_import",
                "wildcard_import",
                "identifier",
                "attribute",
            ],
            &["name", "alias", "module_name", "superclasses"],
        ),
        // go_import_symbols, go_callee, call_qualifier
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
            &["path", "name", "field", "operand", "type", "function"],
        ),
        // read_declaration_visibility, call_qualifier, import_symbols,
        // module_name
        Language::Java => (
            &[
                "modifiers",
                "asterisk",
                "scoped_identifier",
                "identifier",
                // type_refs
                "superclass",
                "super_interfaces",
                "extends_interfaces",
                "type_list",
            ],
            &["object"],
        ),
        // read_declaration_visibility, csharp_using_symbols
        Language::CSharp => (
            &[
                "modifier",
                "identifier",
                "qualified_name",
                "generic_name",
                "alias_qualified_name",
                // type_refs
                "base_list",
                "comment",
            ],
            &["name"],
        ),
        // kotlin_callee, call_qualifier, import_module_path, kotlin_node_name
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
                // type_refs
                "delegation_specifiers",
                "user_type",
                "constructor_invocation",
                "explicit_delegation",
            ],
            // kotlin_node_name: `type_alias` name, `companion_object` name
            &["type", "name"],
        ),
        // swift_callee, call_qualifier
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
                // type_refs
                "inheritance_specifier",
            ],
            // call_qualifier: `target`; type_refs: `inherits_from`
            &[
                "constructed_type",
                "suffix",
                "declaration_kind",
                "target",
                "inherits_from",
            ],
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
                // type_refs
                "base_class_clause",
                "access_specifier",
                "comment",
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
                // type_refs
                "base_clause",
                "class_interface_clause",
                "comment",
            ],
            // call_qualifier: `object`, `scope`
            &["body", "alias", "object", "scope"],
        ),
        // ruby_callee, call_qualifier, ruby_require_path
        Language::Ruby => (
            &[
                "constant",
                "scope_resolution",
                "string",
                "string_content",
                // type_refs
                "superclass",
                "comment",
            ],
            &["method", "receiver", "name", "arguments"],
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

        let bad = bad_names(language);
        if !bad.is_empty() {
            failures.push(format!(
                "{language:?}: names missing from grammar (fix the mapping): {bad:?}"
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

//! Types named in source: supertypes (`Extends` / `Implements` refs) and
//! explicit constructions (`Instantiates` refs).
//!
//! Every node kind and field name used here is listed in the grammar guard's
//! `helper_names` (`grammar_guard_tests.rs`).

use tree_sitter::Node as TsNode;

use super::child_of_kind;
use crate::types::{EdgeKind, Language};

/// A type named in source, e.g. `app.Shape<T>` in `extends app.Shape<T>`:
/// name `Shape`, qualifier `app`.
pub(super) struct TypeRef<'tree> {
    pub name: String,
    pub qualifier: Option<String>,
    pub kind: EdgeKind,
    /// Where the type is written (position of the ref).
    pub node: TsNode<'tree>,
}

/// Supertypes named by the type declaration `decl`.
///
/// Explicit `implements` clauses (Java, TypeScript, PHP) yield `Implements`;
/// everything else yields `Extends`, including clauses that mix both (Kotlin
/// delegation specifiers, C# base lists, Swift inheritance specifiers, C++
/// base classes). The resolver turns `Extends` of an interface / protocol /
/// trait by a non-interface type into `Implements`.
pub(super) fn supertype_refs<'tree>(
    decl: &TsNode<'tree>,
    source: &str,
    language: Language,
) -> Vec<TypeRef<'tree>> {
    let mut types: Vec<(TsNode<'tree>, EdgeKind)> = Vec::new();
    if language == Language::Python {
        // `class C(Base, metaclass=M)`: positional arguments only.
        if let Some(arguments) = decl.child_by_field_name("superclasses") {
            types.extend(
                arguments
                    .named_children(&mut arguments.walk())
                    .filter(|arg| matches!(arg.kind(), "identifier" | "attribute"))
                    .map(|arg| (arg, EdgeKind::Extends)),
            );
        }
    }
    for clause in decl.children(&mut decl.walk()) {
        let extends = EdgeKind::Extends;
        match (language, clause.kind()) {
            (Language::Kotlin, "delegation_specifiers") => types.extend(
                clause
                    .named_children(&mut clause.walk())
                    .filter_map(|spec| kotlin_delegated_type(&spec))
                    .map(|t| (t, extends)),
            ),
            (Language::Java, "superclass") => {
                types.extend(clause.named_child(0).map(|t| (t, extends)));
            }
            (Language::Java, "super_interfaces") => {
                types.extend(type_list(&clause).map(|t| (t, EdgeKind::Implements)));
            }
            (Language::Java, "extends_interfaces") => {
                types.extend(type_list(&clause).map(|t| (t, extends)));
            }
            (
                Language::JavaScript | Language::Jsx | Language::TypeScript | Language::Tsx,
                "class_heritage",
            ) => {
                for part in clause.named_children(&mut clause.walk()) {
                    match part.kind() {
                        "extends_clause" => types.extend(
                            part.children_by_field_name("value", &mut part.walk())
                                .map(|t| (t, extends)),
                        ),
                        "implements_clause" => types.extend(
                            part.named_children(&mut part.walk())
                                .map(|t| (t, EdgeKind::Implements)),
                        ),
                        // JavaScript: `extends <expression>`
                        _ => types.push((part, extends)),
                    }
                }
            }
            (Language::TypeScript | Language::Tsx, "extends_type_clause") => types.extend(
                clause
                    .children_by_field_name("type", &mut clause.walk())
                    .map(|t| (t, extends)),
            ),
            (Language::Swift, "inheritance_specifier") => {
                types.extend(
                    clause
                        .child_by_field_name("inherits_from")
                        .map(|t| (t, extends)),
                );
            }
            (Language::CSharp, "base_list")
            | (Language::Ruby, "superclass")
            | (Language::Php, "base_clause")
            | (Language::Cpp, "base_class_clause") => types.extend(
                clause
                    .named_children(&mut clause.walk())
                    .filter(|t| {
                        !matches!(
                            t.kind(),
                            "access_specifier" | "attribute_declaration" | "comment"
                        )
                    })
                    .map(|t| (t, extends)),
            ),
            (Language::Php, "class_interface_clause") => types.extend(
                clause
                    .named_children(&mut clause.walk())
                    .map(|t| (t, EdgeKind::Implements)),
            ),
            _ => {}
        }
    }
    types
        .into_iter()
        .filter_map(|(node, kind)| type_ref(node, source, kind))
        .collect()
}

/// Type explicitly constructed by `node`: `new Foo()` (Java, C#, JS / TS,
/// PHP, C++), `Foo<T>()` (Swift `constructor_expression`), `Foo.new` /
/// `A::Foo.new` (Ruby), `Foo{…}` / `pkg.Foo{…}` (Go composite literals,
/// Rust struct expressions). Calls that may construct (`Circle(2.0)` in
/// Kotlin, Python, Swift) are recorded as calls; the resolver turns them
/// into instantiations when the callee is a type.
pub(super) fn instantiation_ref<'tree>(
    node: &TsNode<'tree>,
    source: &str,
    language: Language,
) -> Option<TypeRef<'tree>> {
    let type_node = match (language, node.kind()) {
        (Language::Java | Language::CSharp, "object_creation_expression")
        | (Language::Cpp, "new_expression") => node.child_by_field_name("type")?,
        (
            Language::JavaScript | Language::Jsx | Language::TypeScript | Language::Tsx,
            "new_expression",
        ) => node.child_by_field_name("constructor")?,
        (Language::Php, "object_creation_expression") => node
            .named_children(&mut node.walk())
            .find(|c| matches!(c.kind(), "name" | "qualified_name"))?,
        (Language::Swift, "constructor_expression") => {
            node.child_by_field_name("constructed_type")?
        }
        (Language::Go, "composite_literal") => node.child_by_field_name("type").filter(|t| {
            matches!(
                t.kind(),
                "type_identifier" | "qualified_type" | "generic_type"
            )
        })?,
        (Language::Rust, "struct_expression") => node.child_by_field_name("name")?,
        (Language::Ruby, "call") => {
            let method = node.child_by_field_name("method")?;
            if method.utf8_text(source.as_bytes()).ok()? != "new" {
                return None;
            }
            node.child_by_field_name("receiver")
                .filter(|r| matches!(r.kind(), "constant" | "scope_resolution"))?
        }
        _ => return None,
    };
    type_ref(type_node, source, EdgeKind::Instantiates)
}

/// Rust `impl Trait for Type`: the implementing type's name and an
/// `Implements` ref to the trait. Inherent impls (`impl Type`) have none.
pub(super) fn rust_trait_impl<'tree>(
    node: &TsNode<'tree>,
    source: &str,
) -> Option<(String, TypeRef<'tree>)> {
    if node.kind() != "impl_item" {
        return None;
    }
    let trait_ref = type_ref(
        node.child_by_field_name("trait")?,
        source,
        EdgeKind::Implements,
    )?;
    let self_type = node
        .child_by_field_name("type")?
        .utf8_text(source.as_bytes())
        .ok()?;
    let (self_name, _) = split_type_name(self_type)?;
    Some((self_name, trait_ref))
}

/// Type named by a Kotlin `delegation_specifier`: `Base(1)`
/// (`constructor_invocation`), `I` (`user_type`) or `I by impl`
/// (`explicit_delegation`). Function types have no name.
fn kotlin_delegated_type<'tree>(spec: &TsNode<'tree>) -> Option<TsNode<'tree>> {
    let inner = spec.named_child(0)?;
    match inner.kind() {
        "user_type" => Some(inner),
        "constructor_invocation" | "explicit_delegation" => child_of_kind(&inner, "user_type"),
        _ => None,
    }
}

/// Types of a Java `super_interfaces` / `extends_interfaces` clause.
fn type_list<'tree>(clause: &TsNode<'tree>) -> impl Iterator<Item = TsNode<'tree>> {
    child_of_kind(clause, "type_list")
        .map(|list| {
            list.named_children(&mut list.walk())
                .collect::<Vec<_>>()
                .into_iter()
        })
        .into_iter()
        .flatten()
}

pub(super) fn type_ref<'tree>(
    node: TsNode<'tree>,
    source: &str,
    kind: EdgeKind,
) -> Option<TypeRef<'tree>> {
    let text = node.utf8_text(source.as_bytes()).ok()?;
    let (name, qualifier) = split_type_name(text)?;
    Some(TypeRef {
        name,
        qualifier,
        kind,
        node,
    })
}

/// `app.a.Shape<T>` → (`Shape`, `app.a`); `A::B(1)` → (`B`, `A`);
/// `\App\Shape` → (`Shape`, `App`). Type arguments, constructor arguments
/// and composite-literal bodies are dropped; anything else that isn't a
/// plain (qualified) name yields `None`.
fn split_type_name(text: &str) -> Option<(String, Option<String>)> {
    let base = text.split(['<', '(', '[', '{']).next()?.trim();
    let base = base
        .trim_start_matches('\\')
        .trim_start_matches("::")
        .trim_end_matches("::");
    let plain = !base.is_empty()
        && base
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | ':' | '\\'));
    if !plain {
        return None;
    }
    let split = ["::", ".", "\\"]
        .iter()
        .filter_map(|sep| base.rfind(sep).map(|at| (at, at + sep.len())))
        .max();
    let (name, qualifier) = match split {
        Some((end, start)) => (base.get(start..)?, base.get(..end)),
        None => (base, None),
    };
    if name.is_empty() {
        return None;
    }
    Some((
        name.to_string(),
        qualifier.filter(|q| !q.is_empty()).map(str::to_string),
    ))
}

#[cfg(test)]
mod tests {
    use super::split_type_name;

    #[test]
    fn type_names_drop_arguments_and_split_qualifiers() {
        let split = |text| split_type_name(text);
        let named = |name: &str, qualifier: Option<&str>| {
            Some((name.to_string(), qualifier.map(str::to_string)))
        };
        assert_eq!(split("Shape"), named("Shape", None));
        assert_eq!(split("app.a.Shape<T>"), named("Shape", Some("app.a")));
        assert_eq!(split("A::B(1)"), named("B", Some("A")));
        assert_eq!(split("\\App\\Shape"), named("Shape", Some("App")));
        assert_eq!(split("Map<String, Int>"), named("Map", None));
        assert_eq!(split("S::<T>"), named("S", None));
        assert_eq!(split("Circle{R: 1}"), named("Circle", None));
        assert_eq!(split("mixin(Base)"), named("mixin", None));
        assert_eq!(split("(a || b)"), None);
        assert_eq!(split("virtual Base"), None);
    }
}

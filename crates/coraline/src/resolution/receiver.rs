//! Call receivers / qualifiers: which declarations `q.name()` may target.
//!
//! Without type information the resolver only knows the qualifier text
//! (`String`, `this`, `c`, `Circle`) and the containers of each candidate
//! (`Circle::Companion` for `a.kt::Circle::Companion::create`). These rules
//! reject candidates the qualifier clearly rules out, so `String.format(..)`
//! inside `fun format` doesn't become a self-edge.

use crate::types::Language;

/// Last segment of a qualifier: `Circle` for `app.Circle`, `sub` for
/// `ext::sub`, `Ext` for `\App\Ext`.
pub fn last_segment(qualifier: &str) -> &str {
    qualifier
        .rsplit(['.', ':', '\\'])
        .find(|segment| !segment.is_empty())
        .unwrap_or(qualifier)
}

/// Receivers naming the enclosing object or type.
pub fn is_self_like(qualifier: &str) -> bool {
    matches!(
        qualifier,
        "this" | "self" | "Self" | "super" | "base" | "cls" | "static" | "parent" | "@"
    )
}

/// Languages whose extractor nests methods under their type, so a function
/// without a container is a free function. Rust `impl` methods, Go methods
/// and out-of-line C++ methods have no container.
const fn tracks_method_containers(language: Language) -> bool {
    matches!(
        language,
        Language::Kotlin
            | Language::Java
            | Language::CSharp
            | Language::Python
            | Language::TypeScript
            | Language::JavaScript
            | Language::Swift
            | Language::Php
            | Language::Ruby
    )
}

/// Whether some container (`Circle`, `App.A`) is the type / namespace the
/// qualifier names.
pub fn names_container<S: AsRef<str>>(qualifier: &str, containers: &[S]) -> bool {
    let name = last_segment(qualifier);
    containers.iter().any(|container| {
        let container = container.as_ref();
        container == name
            || container
                .strip_suffix(name)
                .is_some_and(|rest| rest.ends_with('.'))
    })
}

/// Whether a call `qualifier.name()` may target a same-named declaration
/// nested in `containers` (outermost first; empty for top-level).
///
/// - a container named like the qualifier (`Circle.create()`) → yes;
/// - `this` / `self` / `super` → only methods;
/// - a capitalised qualifier (`String.format`) names another type → no;
/// - a lower-case qualifier is a variable of unknown type → any method, but
///   free functions only for Kotlin (extension functions).
///
/// Where free functions and methods look alike (Rust, Go, C/C++ without a
/// container) the declaration is admitted unless the qualifier names a
/// different container.
pub fn qualifier_admits<S: AsRef<str>>(
    language: Language,
    qualifier: &str,
    containers: &[S],
) -> bool {
    if names_container(qualifier, containers) {
        return true;
    }
    if containers.is_empty() && !tracks_method_containers(language) {
        return true;
    }
    let name = last_segment(qualifier);
    if is_self_like(name) {
        return !containers.is_empty();
    }
    if name.chars().next().is_some_and(char::is_uppercase) {
        return false;
    }
    !containers.is_empty() || language == Language::Kotlin
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: [&str; 0] = [];

    #[test]
    fn capitalised_qualifiers_need_a_matching_container() {
        assert!(!qualifier_admits(Language::Kotlin, "String", &NONE));
        assert!(!qualifier_admits(Language::Java, "String", &["Fmt"]));
        assert!(qualifier_admits(Language::Java, "Fmt", &["Fmt"]));
        assert!(qualifier_admits(
            Language::Kotlin,
            "Circle",
            &["Circle", "Companion"]
        ));
        assert!(qualifier_admits(Language::CSharp, "A", &["App.A"]));
        assert!(qualifier_admits(Language::Php, "\\App\\Ext", &["Ext"]));
    }

    #[test]
    fn self_and_variables_admit_methods() {
        assert!(qualifier_admits(Language::Java, "this", &["A"]));
        assert!(!qualifier_admits(Language::Python, "self", &NONE));
        assert!(qualifier_admits(Language::TypeScript, "c", &["Circle"]));
        assert!(!qualifier_admits(Language::Python, "c", &NONE));
        assert!(qualifier_admits(Language::Kotlin, "s", &NONE));
    }

    #[test]
    fn untracked_containers_are_admitted() {
        assert!(qualifier_admits(Language::Rust, "Circle", &NONE));
        assert!(qualifier_admits(Language::Go, "c", &NONE));
        assert!(!qualifier_admits(Language::Cpp, "Other", &["Shape"]));
    }
}

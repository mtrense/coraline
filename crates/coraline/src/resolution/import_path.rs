//! Map import module paths to project files.
//!
//! Imports name their target in language-specific syntax (`com.example.Foo`,
//! `crate::a::b`, `App\Model\Circle`, `../a/report`, `.shapes`, `"x.h"`).
//! [`import_targets`] turns such a path into project-relative path segments
//! and [`match_score`] checks a candidate file against them.

use std::path::Path;

use crate::types::Language;

/// Location an import refers to, as `/`-separated path segments without
/// file extension.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportTarget {
    pub segments: Vec<String>,
    /// Resolved relative to the importing file (`./x`, `.x`, `crate::x`,
    /// `"x.h"`): must match the candidate path exactly. Otherwise the
    /// target is matched as a path suffix (source roots and module prefixes
    /// such as `src/main/java` or `example.com/app` are unknown).
    pub anchored: bool,
}

/// Score of an anchored (exact) match; suffix matches score their number of
/// matching segments.
pub const EXACT_SCORE: usize = 1000;

/// File extensions stripped from slash-separated import paths.
const SOURCE_EXTENSIONS: &[&str] = &[
    "d.ts", "ts", "tsx", "js", "jsx", "mjs", "cjs", "mts", "cts", "h", "hh", "hpp", "hxx", "c",
    "cc", "cpp", "cxx", "rs", "py", "go", "java", "kt", "kts", "cs", "swift", "php", "rb",
];

/// Targets of `module_path` imported from `from_file` (project-relative).
///
/// `item` is the imported symbol name when known (the callee or the
/// import's `export=` name): if the path ends with it (`app.a.describe`,
/// `crate::a::report::describe`), the path without the item is a target too.
pub fn import_targets(
    module_path: &str,
    language: Language,
    from_file: &str,
    item: Option<&str>,
) -> Vec<ImportTarget> {
    let mut targets = base_targets(module_path.trim(), language, from_file);

    if let Some(item) = item {
        let without_item: Vec<ImportTarget> = targets
            .iter()
            .filter(|t| t.segments.len() > 1 && t.segments.last().is_some_and(|s| s == item))
            .map(|t| ImportTarget {
                segments: t
                    .segments
                    .iter()
                    .take(t.segments.len() - 1)
                    .cloned()
                    .collect(),
                anchored: t.anchored,
            })
            .collect();
        targets.extend(without_item);
    }

    targets.retain(|t| !t.segments.is_empty());
    targets.dedup();
    targets
}

fn base_targets(module_path: &str, language: Language, from_file: &str) -> Vec<ImportTarget> {
    let from_dir = parent_segments(from_file);
    match language {
        Language::JavaScript | Language::Jsx | Language::TypeScript | Language::Tsx => {
            if module_path.starts_with('.') {
                vec![anchored(join(&from_dir, &slash_segments(module_path)))]
            } else {
                vec![unanchored(slash_segments(module_path))]
            }
        }
        Language::Python => python_targets(module_path, &from_dir),
        Language::Rust => rust_targets(module_path, from_file),
        Language::C | Language::Cpp => c_targets(module_path, &from_dir),
        Language::Php => vec![unanchored(split_segments(module_path, '\\'))],
        Language::Go => vec![unanchored(slash_segments(module_path))],
        _ => {
            if module_path.contains('/') {
                vec![unanchored(slash_segments(module_path))]
            } else {
                vec![unanchored(split_segments(module_path, '.'))]
            }
        }
    }
}

/// `.shapes` / `..core.base` are relative to the importing file's package;
/// `a.report` is absolute.
fn python_targets(module_path: &str, from_dir: &[String]) -> Vec<ImportTarget> {
    let rest = module_path.trim_start_matches('.');
    let dots = module_path.len() - rest.len();
    let segments = split_segments(rest, '.');
    if dots == 0 {
        return vec![unanchored(segments)];
    }
    let up = dots - 1;
    if up > from_dir.len() {
        return Vec::new();
    }
    let mut base: Vec<String> = from_dir.iter().take(from_dir.len() - up).cloned().collect();
    base.extend(segments);
    vec![anchored(base)]
}

/// `crate::` is relative to the crate's `src` dir, `self::` / `super::` to
/// the importing file's module; other paths (external crates, 2015-style
/// paths) are matched as suffixes.
fn rust_targets(module_path: &str, from_file: &str) -> Vec<ImportTarget> {
    let mut segments = split_segments(module_path.trim_start_matches("::"), ':');
    let Some(first) = segments.first().cloned() else {
        return Vec::new();
    };
    match first.as_str() {
        "crate" => {
            segments.remove(0);
            let dir = parent_segments(from_file);
            match dir.iter().rposition(|s| s == "src") {
                Some(src) => {
                    let mut base: Vec<String> = dir.iter().take(src + 1).cloned().collect();
                    base.extend(segments);
                    vec![anchored(base)]
                }
                None => vec![unanchored(segments)],
            }
        }
        "self" | "super" => {
            let mut base = rust_module_segments(from_file);
            let mut rest = segments.as_slice();
            if let [head, tail @ ..] = rest
                && head == "self"
            {
                rest = tail;
            }
            while let [head, tail @ ..] = rest {
                if head != "super" {
                    break;
                }
                if base.pop().is_none() {
                    return Vec::new();
                }
                rest = tail;
            }
            base.extend(rest.iter().cloned());
            vec![anchored(base)]
        }
        _ => vec![unanchored(segments)],
    }
}

/// Module path of a Rust file: `src/a/report.rs` → `src/a/report`;
/// `mod.rs` / `lib.rs` / `main.rs` stand for their directory.
fn rust_module_segments(file: &str) -> Vec<String> {
    let mut segments = file_segments(file);
    if segments
        .last()
        .is_some_and(|s| matches!(s.as_str(), "mod" | "lib" | "main"))
    {
        segments.pop();
    }
    segments
}

/// `"x.h"` is relative to the including file (or an include dir, matched as
/// suffix when it names a directory too); `<x.h>` is a system/include-dir
/// header.
fn c_targets(module_path: &str, from_dir: &[String]) -> Vec<ImportTarget> {
    if module_path.starts_with('<') {
        let inner = module_path.trim_start_matches('<').trim_end_matches('>');
        return vec![unanchored(slash_segments(inner))];
    }
    let inner = module_path.trim_matches('"');
    let segments = slash_segments(inner);
    let mut targets = vec![anchored(join(from_dir, &segments))];
    if segments.len() > 1 {
        targets.push(unanchored(segments));
    }
    targets
}

const fn anchored(segments: Vec<String>) -> ImportTarget {
    ImportTarget {
        segments,
        anchored: true,
    }
}

const fn unanchored(segments: Vec<String>) -> ImportTarget {
    ImportTarget {
        segments,
        anchored: false,
    }
}

fn split_segments(path: &str, separator: char) -> Vec<String> {
    path.split(separator)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// `/`-separated path without its file extension.
fn slash_segments(path: &str) -> Vec<String> {
    let mut segments = split_segments(&path.replace('\\', "/"), '/');
    if let Some(last) = segments.last_mut() {
        *last = strip_source_extension(last).to_string();
    }
    segments
}

fn strip_source_extension(name: &str) -> &str {
    SOURCE_EXTENSIONS
        .iter()
        .find_map(|ext| {
            name.strip_suffix(ext)
                .and_then(|rest| rest.strip_suffix('.'))
                .filter(|rest| !rest.is_empty())
        })
        .unwrap_or(name)
}

/// `dir` joined with relative `segments`, resolving `.` and `..`.
fn join(dir: &[String], segments: &[String]) -> Vec<String> {
    let mut result = dir.to_vec();
    for segment in segments {
        match segment.as_str() {
            "." => {}
            ".." => {
                result.pop();
            }
            _ => result.push(segment.clone()),
        }
    }
    result
}

/// Segments of a project-relative file path, extension stripped from the
/// file name.
fn file_segments(file: &str) -> Vec<String> {
    let normalized = file.replace('\\', "/");
    let mut segments = split_segments(&normalized, '/');
    if let Some(last) = segments.last_mut()
        && let Some(stem) = Path::new(last.as_str())
            .file_stem()
            .and_then(|s| s.to_str())
    {
        *last = stem.to_string();
    }
    segments
}

fn parent_segments(file: &str) -> Vec<String> {
    let mut segments = split_segments(&file.replace('\\', "/"), '/');
    segments.pop();
    segments
}

/// Module paths a file can be imported as: the file itself and, for package
/// entry files (`index.ts`, `__init__.py`, `mod.rs`, `lib.rs`), its dir.
fn file_forms(file: &str) -> Vec<Vec<String>> {
    let segments = file_segments(file);
    let mut forms = vec![segments.clone()];
    if segments
        .last()
        .is_some_and(|s| matches!(s.as_str(), "index" | "__init__" | "mod" | "lib" | "main"))
    {
        forms.push(segments.iter().take(segments.len() - 1).cloned().collect());
    }
    forms
}

/// How well `file` matches `target`: [`EXACT_SCORE`] for an anchored match,
/// the number of matching trailing segments for a suffix match, `None` if
/// the file isn't the target.
///
/// A suffix match must cover the whole target, the whole file path, or at
/// least two segments, so `utils` matches `src/utils.py` but `app/a/Report`
/// does not match `other/Report.java`.
pub fn match_score(file: &str, target: &ImportTarget) -> Option<usize> {
    file_forms(file)
        .iter()
        .filter_map(|form| segments_score(form, target))
        .max()
}

/// [`match_score`] for the directory of `file` (Go packages) instead of the
/// file itself.
pub fn dir_match_score(file: &str, target: &ImportTarget) -> Option<usize> {
    segments_score(&parent_segments(file), target)
}

fn segments_score(form: &[String], target: &ImportTarget) -> Option<usize> {
    if target.anchored {
        return (form == target.segments.as_slice()).then_some(EXACT_SCORE);
    }
    let common = form
        .iter()
        .rev()
        .zip(target.segments.iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    (common > 0 && (common == target.segments.len() || common == form.len() || common >= 2))
        .then_some(common)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segs(path: &str) -> Vec<String> {
        split_segments(path, '/')
    }

    fn targets(module_path: &str, language: Language, from: &str) -> Vec<(String, bool)> {
        import_targets(module_path, language, from, None)
            .into_iter()
            .map(|t| (t.segments.join("/"), t.anchored))
            .collect()
    }

    #[test]
    fn dotted_and_backslash_paths_become_slash_paths() {
        assert_eq!(
            targets("com.example.b.Helper", Language::Java, "b/App.java"),
            vec![("com/example/b/Helper".to_string(), false)]
        );
        assert_eq!(
            targets("app.a.describe", Language::Kotlin, "b/App.kt"),
            vec![("app/a/describe".to_string(), false)]
        );
        assert_eq!(
            targets("App\\Model\\Circle", Language::Php, "b/App.php"),
            vec![("App/Model/Circle".to_string(), false)]
        );
        assert_eq!(
            targets("example.com/app/a", Language::Go, "b/app.go"),
            vec![("example.com/app/a".to_string(), false)]
        );
    }

    #[test]
    fn relative_paths_resolve_against_the_importing_file() {
        assert_eq!(
            targets("../a/report", Language::TypeScript, "b/app.ts"),
            vec![("a/report".to_string(), true)]
        );
        assert_eq!(
            targets("./shapes.js", Language::JavaScript, "a/report.js"),
            vec![("a/shapes".to_string(), true)]
        );
        assert_eq!(
            targets("\"../a/report.h\"", Language::Cpp, "b/app.cpp"),
            vec![
                ("a/report".to_string(), true),
                ("../a/report".to_string(), false)
            ]
        );
        assert_eq!(
            targets("shapes.h", Language::C, "a/report.c"),
            vec![("a/shapes".to_string(), true)]
        );
        assert_eq!(
            targets("<stdio.h>", Language::C, "a/report.c"),
            vec![("stdio".to_string(), false)]
        );
    }

    #[test]
    fn python_relative_imports_resolve_against_the_package() {
        assert_eq!(
            targets(".shapes", Language::Python, "pkg/a/report.py"),
            vec![("pkg/a/shapes".to_string(), true)]
        );
        assert_eq!(
            targets("..core.base", Language::Python, "pkg/a/report.py"),
            vec![("pkg/core/base".to_string(), true)]
        );
        assert_eq!(
            targets(".", Language::Python, "pkg/a/report.py"),
            vec![("pkg/a".to_string(), true)]
        );
        assert_eq!(
            targets("a.report", Language::Python, "b/app.py"),
            vec![("a/report".to_string(), false)]
        );
        assert_eq!(targets("...x", Language::Python, "a.py"), Vec::new());
    }

    #[test]
    fn rust_paths_resolve_crate_self_and_super() {
        assert_eq!(
            targets(
                "crate::a::report::describe",
                Language::Rust,
                "x/src/b/app.rs"
            ),
            vec![("x/src/a/report/describe".to_string(), true)]
        );
        assert_eq!(
            targets("super::shapes::square", Language::Rust, "src/a/report.rs"),
            vec![("src/a/shapes/square".to_string(), true)]
        );
        assert_eq!(
            targets("super::shapes", Language::Rust, "src/a/mod.rs"),
            vec![("src/shapes".to_string(), true)]
        );
        assert_eq!(
            targets("self::inner::f", Language::Rust, "src/a/mod.rs"),
            vec![("src/a/inner/f".to_string(), true)]
        );
        assert_eq!(
            targets("serde::Serialize", Language::Rust, "src/a.rs"),
            vec![("serde/Serialize".to_string(), false)]
        );
    }

    #[test]
    fn item_suffix_adds_the_containing_module() {
        let found: Vec<String> = import_targets(
            "crate::a::report::describe",
            Language::Rust,
            "src/b/app.rs",
            Some("describe"),
        )
        .into_iter()
        .map(|t| t.segments.join("/"))
        .collect();
        assert_eq!(found, vec!["src/a/report/describe", "src/a/report"]);
    }

    #[test]
    fn suffix_matches_need_enough_overlap() {
        let java = unanchored(segs("app/a/Report"));
        assert_eq!(match_score("a/Report.java", &java), Some(2));
        assert_eq!(
            match_score("src/main/java/app/a/Report.java", &java),
            Some(3)
        );
        assert_eq!(match_score("other/Report.java", &java), None);
        let utils = unanchored(segs("utils"));
        assert_eq!(match_score("src/utils.py", &utils), Some(1));
        let go = unanchored(segs("example.com/app/a"));
        assert_eq!(dir_match_score("a/report.go", &go), Some(1));
        assert_eq!(dir_match_score("pkg/b/a/report.go", &go), None);
    }

    #[test]
    fn anchored_matches_are_exact_and_know_package_entry_files() {
        let report = anchored(segs("a/report"));
        assert_eq!(match_score("a/report.ts", &report), Some(EXACT_SCORE));
        assert_eq!(match_score("x/a/report.ts", &report), None);
        let package = anchored(segs("a/util"));
        assert_eq!(match_score("a/util/index.ts", &package), Some(EXACT_SCORE));
        assert_eq!(
            match_score("a/util/__init__.py", &package),
            Some(EXACT_SCORE)
        );
        assert_eq!(match_score("a\\util\\mod.rs", &package), Some(EXACT_SCORE));
    }
}

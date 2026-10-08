//! Swift modules: which files see each other's declarations without an
//! import.
//!
//! Swift files have no per-file imports within a module, so a call to a
//! function declared in another directory is legal when both files belong
//! to the same module. Modules are only recognised from build manifests, so
//! unrelated projects in one workspace never share one (#43):
//!
//! - Swift Package Manager: a directory with `Package.swift`; each
//!   `Sources/<Target>/` and `Tests/<Target>/` subtree is a module named
//!   `<Target>`. Files elsewhere in the package belong to no module
//!   (custom target `path:`s aren't parsed).
//! - Xcode: a directory containing a `*.xcodeproj` is one module (its
//!   targets aren't parsed, so they are merged).
//!
//! The nearest manifest above a file decides. Files without one belong to
//! no module and only see same-file / same-directory declarations.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

/// A Swift module a file belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwiftModule {
    /// Project-relative directory of the package / Xcode project.
    pub root: String,
    /// SPM target name (`Sources/<Target>/`); `None` for Xcode projects.
    pub target: Option<String>,
}

/// Swift module per file, read from the file system and cached.
#[derive(Debug)]
pub struct SwiftModules {
    project_root: PathBuf,
    by_file: HashMap<String, Option<SwiftModule>>,
    /// Manifest found directly in a directory, per project-relative dir.
    manifests: HashMap<PathBuf, Option<Manifest>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Manifest {
    Package,
    Xcode,
}

impl SwiftModules {
    pub fn new(project_root: &Path) -> Self {
        Self {
            project_root: project_root.to_path_buf(),
            by_file: HashMap::new(),
            manifests: HashMap::new(),
        }
    }

    /// Module of the project-relative `file_path`, if any.
    pub fn module(&mut self, file_path: &str) -> Option<&SwiftModule> {
        if !self.by_file.contains_key(file_path) {
            let module = self.detect(file_path);
            self.by_file.insert(file_path.to_string(), module);
        }
        self.by_file.get(file_path).and_then(Option::as_ref)
    }

    fn detect(&mut self, file_path: &str) -> Option<SwiftModule> {
        let file = PathBuf::from(file_path.replace('\\', "/"));
        let mut dir = file.parent();
        while let Some(current) = dir {
            match self.manifest(current) {
                Some(Manifest::Package) => {
                    let rest: Vec<&str> = file
                        .strip_prefix(current)
                        .ok()?
                        .components()
                        .filter_map(|c| match c {
                            Component::Normal(s) => s.to_str(),
                            _ => None,
                        })
                        .collect();
                    // `Sources/<Target>/…/File.swift`
                    return match rest.as_slice() {
                        [top, target, _, ..] if matches!(*top, "Sources" | "Tests") => {
                            Some(SwiftModule {
                                root: dir_string(current),
                                target: Some((*target).to_string()),
                            })
                        }
                        _ => None,
                    };
                }
                Some(Manifest::Xcode) => {
                    return Some(SwiftModule {
                        root: dir_string(current),
                        target: None,
                    });
                }
                None => dir = current.parent(),
            }
        }
        None
    }

    fn manifest(&mut self, dir: &Path) -> Option<Manifest> {
        if let Some(manifest) = self.manifests.get(dir) {
            return *manifest;
        }
        let abs = self.project_root.join(dir);
        let manifest = if abs.join("Package.swift").is_file() {
            Some(Manifest::Package)
        } else if std::fs::read_dir(&abs).is_ok_and(|entries| {
            entries.filter_map(Result::ok).any(|entry| {
                entry
                    .path()
                    .extension()
                    .is_some_and(|ext| ext == "xcodeproj")
                    && entry.path().is_dir()
            })
        }) {
            Some(Manifest::Xcode)
        } else {
            None
        };
        self.manifests.insert(dir.to_path_buf(), manifest);
        manifest
    }
}

fn dir_string(dir: &Path) -> String {
    dir.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;

    fn modules(files: &[&str]) -> (tempfile::TempDir, SwiftModules) {
        let temp = tempfile::TempDir::new().expect("temp dir");
        for file in files {
            let path = temp.path().join(file);
            std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
            std::fs::write(path, "").expect("write");
        }
        let modules = SwiftModules::new(temp.path());
        (temp, modules)
    }

    fn module(root: &str, target: Option<&str>) -> SwiftModule {
        SwiftModule {
            root: root.to_string(),
            target: target.map(str::to_string),
        }
    }

    #[test]
    fn spm_targets_are_modules() {
        let (_temp, mut modules) = modules(&["pkg/Package.swift", "Package.swift"]);
        assert_eq!(
            modules.module("pkg/Sources/App/a/x.swift"),
            Some(&module("pkg", Some("App")))
        );
        assert_eq!(
            modules.module("pkg/Tests/AppTests/x.swift"),
            Some(&module("pkg", Some("AppTests")))
        );
        assert_eq!(
            modules.module("Sources/Lib/x.swift"),
            Some(&module("", Some("Lib")))
        );
        // Not under `Sources/<Target>/`.
        assert_eq!(modules.module("pkg/Scripts/x.swift"), None);
        assert_eq!(modules.module("pkg/Sources/x.swift"), None);
    }

    #[test]
    fn xcode_projects_are_modules() {
        let (_temp, mut modules) = modules(&["ios/App.xcodeproj/project.pbxproj"]);
        assert_eq!(
            modules.module("ios/App/Views/x.swift"),
            Some(&module("ios", None))
        );
        assert_eq!(modules.module("other/x.swift"), None);
    }
}

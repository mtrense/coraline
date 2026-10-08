//! PHP extraction fixture tests.

mod common;

use std::collections::BTreeSet;

use common::{assert_contains_all, call_pairs, import_set, index_project, instantiation_pairs};

#[test]
fn php_calls_are_extracted() {
    let temp = index_project(&[(
        "src/Service.php",
        "<?php\n\
         class Svc {\n\
         \x20   public function build() {\n\
         \x20       helper();\n\
         \x20       $this->run();\n\
         \x20       $obj?->safe();\n\
         \x20       Util::make();\n\
         \x20       \\App\\Util\\fmt();\n\
         \x20       new Circle();\n\
         \x20       new \\App\\Model\\Square();\n\
         \x20   }\n\
         }\n",
    )]);

    assert_contains_all(
        &call_pairs(temp.path()),
        &[
            "build -> helper",
            "build -> run",
            "build -> safe",
            "build -> make",
            "build -> fmt",
        ],
    );
    assert_contains_all(
        &instantiation_pairs(temp.path()),
        &["build -> Circle", "build -> Square"],
    );
}

#[test]
fn php_use_imports_are_extracted() {
    let temp = index_project(&[(
        "src/Service.php",
        "<?php\n\
         namespace App\\Service;\n\
         \n\
         use App\\Model\\Circle;\n\
         use App\\Model\\{Square, Point as P};\n\
         use function App\\Util\\fmt;\n\
         use App\\Model\\Shape as S, \\App\\Other;\n\
         \n\
         class Svc {\n\
         \x20   use LogTrait;\n\
         }\n",
    )]);

    let expected: BTreeSet<String> = [
        "Circle | App\\Model\\Circle|export=Circle",
        "Square | App\\Model\\Square|export=Square",
        "P | App\\Model\\Point|export=Point",
        "fmt | App\\Util\\fmt|export=fmt",
        "S | App\\Model\\Shape|export=Shape",
        "Other | App\\Other|export=Other",
    ]
    .iter()
    .map(|s| (*s).to_string())
    .collect();
    assert_eq!(import_set(temp.path()), expected);
}

//! Ruby extraction fixture tests.

mod common;

use common::{
    assert_contains_all, assert_contains_none, call_pairs, import_set, index_project,
    instantiation_pairs, node_set,
};

#[test]
fn ruby_calls_are_extracted() {
    let temp = index_project(&[(
        "lib/shapes.rb",
        "module Shapes\n\
         \x20 class Builder < Base\n\
         \x20   def build\n\
         \x20     helper2(1)\n\
         \x20     self.run\n\
         \x20     a.b.deep(1)\n\
         \x20     Circle.new(1)\n\
         \x20     Geo::Square.new\n\
         \x20     list.map { |x| x }.select(&:ok)\n\
         \x20     obj&.safe\n\
         \x20   end\n\
         \n\
         \x20   def self.make\n\
         \x20     create(2)\n\
         \x20   end\n\
         \x20 end\n\
         end\n",
    )]);

    assert_contains_all(
        &node_set(temp.path()),
        &[
            "namespace:Shapes",
            "class:Builder",
            "method:build",
            "method:make",
        ],
    );
    assert_contains_all(
        &call_pairs(temp.path()),
        &[
            "build -> helper2",
            "build -> run",
            "build -> deep",
            "build -> map",
            "build -> select",
            "build -> safe",
            "make -> create",
        ],
    );
    assert_contains_all(
        &instantiation_pairs(temp.path()),
        &["build -> Circle", "build -> Square"],
    );
}

/// `require` / `require_relative` with a literal path are imports (named
/// after the file, relative paths kept relative), not calls.
#[test]
fn ruby_requires_are_imports() {
    let temp = index_project(&[(
        "lib/app.rb",
        "require 'json'\n\
         require \"app/util\"\n\
         require_relative 'shapes'\n\
         require_relative \"../a/report.rb\"\n\
         require(name)\n\
         \n\
         def run\n\
         \x20 loader.require 'x'\n\
         end\n",
    )]);

    let imports = import_set(temp.path());
    assert_contains_all(
        &imports,
        &[
            "json | json",
            "util | app/util",
            "shapes | ./shapes",
            "report | ../a/report.rb",
        ],
    );
    assert_eq!(imports.len(), 4, "{imports:#?}");
    let calls = call_pairs(temp.path());
    assert_contains_none(&calls, &["run -> require_relative"]);
    assert_contains_all(&calls, &["run -> require"]);
}

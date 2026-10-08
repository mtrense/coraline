//! Ruby extraction fixture tests.

mod common;

use common::{assert_contains_all, call_pairs, index_project, node_set};

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
            "build -> Circle",
            "build -> Square",
            "build -> map",
            "build -> select",
            "build -> safe",
            "make -> create",
        ],
    );
}

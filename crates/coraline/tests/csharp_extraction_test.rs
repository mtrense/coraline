//! C# extraction fixture tests.

mod common;

use common::{assert_contains_all, import_set, index_project, query_set};

#[test]
fn csharp_using_directives_are_imports() {
    let temp = index_project(&[(
        "src/Service.cs",
        "using System;\n\
         using System.Collections.Generic;\n\
         using static System.Math;\n\
         using Alias = Example.Model.Circle;\n\
         using L = System.Collections.Generic.List<int>;\n\
         global using Foo.Bar;\n\
         namespace N { using Inner.Ns; class C {} }\n",
    )]);

    let imports = import_set(temp.path());
    assert_contains_all(
        &imports,
        &[
            "System | System|export=System",
            "Generic | System.Collections.Generic|export=Generic",
            "Math | System.Math|export=Math",
            "Alias | Example.Model.Circle|export=Circle",
            "L | System.Collections.Generic.List<int>|export=List",
            "Bar | Foo.Bar|export=Bar",
            "Ns | Inner.Ns|export=Ns",
        ],
    );
    assert_eq!(imports.len(), 7, "{imports:#?}");
}

#[test]
fn csharp_public_declarations_are_exported() {
    let temp = index_project(&[(
        "src/Model.cs",
        "namespace M {\n\
         \x20   public class Circle {\n\
         \x20       public double Area() { return 1; }\n\
         \x20       private void Hidden() {}\n\
         \x20   }\n\
         \x20   internal class Inner {}\n\
         }\n",
    )]);

    let exported = query_set(
        temp.path(),
        "SELECT kind || ':' || name FROM nodes WHERE is_exported = 1",
    );
    assert_eq!(
        exported.into_iter().collect::<Vec<_>>(),
        ["class:Circle", "method:Area"]
    );
}

#[test]
fn csharp_file_scoped_namespace_is_extracted() {
    let temp = index_project(&[(
        "src/Model.cs",
        "namespace App.Model;\n\
         \n\
         public class Circle {}\n",
    )]);

    let namespaces = query_set(
        temp.path(),
        "SELECT name FROM nodes WHERE kind = 'namespace'",
    );
    assert_contains_all(&namespaces, &["App.Model"]);
}

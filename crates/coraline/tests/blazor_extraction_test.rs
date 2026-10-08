//! Blazor files are indexed at file level only: `.razor` is Razor markup, not
//! C#, and there is no usable grammar for it. Code-behind `.cs` files are
//! extracted as C#.

mod common;

use common::{assert_contains_all, index_project, query_set};

#[test]
fn razor_files_get_file_nodes_only() {
    let temp = index_project(&[
        (
            "Pages/Counter.razor",
            "@page \"/counter\"\n\
             <button @onclick=\"IncrementCount\">Click</button>\n\
             @code {\n\
             \x20   private int count = 0;\n\
             \x20   private void IncrementCount() { count++; }\n\
             }\n",
        ),
        (
            "Services/UserService.cs",
            "namespace App.Services;\n\
             public class UserService { public void Load() {} }\n",
        ),
    ]);

    let razor_nodes = query_set(
        temp.path(),
        "SELECT kind || ':' || name FROM nodes WHERE file_path LIKE '%.razor'",
    );
    assert_eq!(
        razor_nodes.into_iter().collect::<Vec<_>>(),
        ["file:Counter.razor"]
    );
    assert_contains_all(
        &query_set(
            temp.path(),
            "SELECT kind || ':' || name FROM nodes WHERE file_path LIKE '%.cs'",
        ),
        &["class:UserService", "method:Load"],
    );
}

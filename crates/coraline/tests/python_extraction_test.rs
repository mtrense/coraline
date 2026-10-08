//! Python extraction fixture tests.

mod common;

use common::{assert_contains_all, import_set, index_project};

#[test]
fn python_imports_are_extracted() {
    let temp = index_project(&[(
        "app/service.py",
        "import os\n\
         import os.path, sys\n\
         import numpy as np\n\
         from app.model import Circle, Point as P, format_num\n\
         from . import util\n\
         from ..core.base import (Base,\n    Mixin)\n\
         from typing import *\n\
         \n\
         def run():\n\
         \x20   return Circle(1)\n",
    )]);

    let imports = import_set(temp.path());
    assert_contains_all(
        &imports,
        &[
            "os | os",
            "os.path | os.path",
            "sys | sys",
            "np | numpy",
            "Circle | app.model|export=Circle",
            "P | app.model|export=Point",
            "format_num | app.model|export=format_num",
            "util | .|export=util",
            "Base | ..core.base|export=Base",
            "Mixin | ..core.base|export=Mixin",
            "* | typing",
        ],
    );
    assert_eq!(imports.len(), 11, "{imports:#?}");
}

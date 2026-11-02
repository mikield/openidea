//! Takes the pure part of upstream's menu definition — `Os`, `Native`, `Entry`,
//! the helpers and `spec()` from `src-tauri/src/menu.rs` — without its Tauri
//! code, so the GPUI client shows the same menu and follows upstream changes.

use std::path::Path;

fn main() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../src-tauri/src/menu.rs");
    println!("cargo:rerun-if-changed={}", source.display());
    let text = std::fs::read_to_string(&source).expect("src-tauri/src/menu.rs");
    let lines: Vec<&str> = text.lines().collect();
    let find = |needle: &str| {
        lines.iter().position(|l| l.starts_with(needle)).unwrap_or_else(|| panic!("menu.rs: no `{needle}`"))
    };
    // Back over the item's doc comments and attributes.
    let item_start = |mut ix: usize| {
        while ix > 0 && (lines[ix - 1].starts_with("///") || lines[ix - 1].starts_with("#[")) {
            ix -= 1;
        }
        ix
    };
    let start = item_start(find("pub enum Os"));
    let end = item_start(find("pub struct MenuRoute"));
    let spec = lines[start..end].join("\n");
    assert!(!spec.contains("tauri::"), "the menu spec started using Tauri: extend the extraction");
    let out = Path::new(&std::env::var("OUT_DIR").unwrap()).join("menu_spec.rs");
    std::fs::write(out, spec).unwrap();
}

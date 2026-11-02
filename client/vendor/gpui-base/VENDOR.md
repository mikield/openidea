# gpui-base 0.8.1, vendored

Source: crates.io `gpui-base` 0.8.1 (https://github.com/longbridge/gpui-kit), Apache-2.0
(`LICENSE-APACHE`). Copied unchanged, then patched; `Cargo.toml.orig` is the published original.
Patched into the client workspace with `[patch.crates-io]`, so `gpui-component` uses it as well.

Why: the editor gate (client/PORTING.md) — React's gutter has lanes (bookmarks, run, changes)
that the published editor cannot lay out.

Our changes (keep this list current; each is marked `// openidea:` in the source):

- `GutterLayout` + `InputBaseState::set_gutter_layout` (state.rs) and its use in element.rs
  (`layout_line_numbers`, `layout_fold_icons`, line-number colours, the border in `paint`):
  a CodeMirror-like gutter — padding around the numbers, room for lanes, fold column width,
  a border line, the gap before the text.

`Cargo.toml` gets an empty `[workspace]` so the crate builds and tests on its own
(`cd vendor/gpui-base && cargo test --features test-support --lib input::`); the client workspace also excludes `vendor/*`.

Updating: copy the new release over this directory, re-apply the changes above, check the list.

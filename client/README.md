# GPUI client (phase-0 spike)

A native Rust UI (GPUI + gpui-component) for upstream's **unmodified** backend, served by our
`oi-server` (`crates/oi-server`: upstream's standard commands plus ours). The client is
remote-first: even a local session spawns `oi-server --stdio`
and speaks the remote-development protocol (`crates/ide-host/src/rpc.rs`), so local
and SSH sessions share one code path.

This is a separate Cargo workspace on purpose: the root `Cargo.toml` / `Cargo.lock`
and everything under `crates/` stay untouched, so upstream merges stay clean.
Upstream crates are used read-only (`ide-core`, `ide-jsonrpc`, `ide-host`, …).

```
client/
├─ crates/client-rpc   typed protocol client over a `Transport` (stdio), UTF-8 ↔ UTF-16 coordinates, e2e tests
├─ crates/oi-server    our backend binary: `Dispatcher::standard()` + `oi_*` commands
├─ crates/client-core  workbench core: plugins and extension points, actions, dock, settings, keymap, theme
├─ crates/plugin-*     features as plugins: editor, project-tree, problems, terminal, search, menu, vcs, presets
└─ crates/client-app   `openidea-gpui`: `main.rs` and the list of plugins
```

## Run

```sh
cd client
cargo build -p oi-server
cargo run -p client-app -- /path/to/project --open src/main.rs
```

The client finds `oi-server` next to its own binary, i.e. in `client/target/{debug,release}`
(or `$OPENIDEA_SERVER`, on `PATH`; an upstream `openidea-server` works too). Linux needs a Vulkan or GL driver; headless:
`scripts/headless-shot.sh` (Xvfb + Mesa llvmpipe). `OPENIDEA_SERVER_HOME` isolates the
backend's config and caches. Plan, decisions and next steps: [`PORTING.md`](PORTING.md).

## What works

- project tree (lazy `fs_read_dir`, refreshed on `FsChanged`)
- open / edit / save documents (`doc_open`, `doc_edit`, `doc_save`); edits are
  diffed against the last synced text and sent in UTF-16 offsets
- diagnostics from the server's language servers (Problems tool window, editor hover)
- the React look: Islands panels, tool window stripes with icons, a frameless title bar
  with project and branch, upstream's icons in the tree and tabs
- a dock: Project (left), Problems (bottom), the editor in the center; Alt+1 / Alt+6 and the
  rest of the chosen keymap; the layout persists per project
- completion through `lsp_completion`, filtered and ranked client-side
- one ordered queue per document: completion never runs against text the
  server has not seen, save never overtakes an edit

`cargo test --workspace` runs `oi-server`'s own end-to-end test, and the client's
end-to-end tests against the real server once `oi-server` is built (as above); the
rust-analyzer test needs `rust-analyzer` on `PATH`.

## Known gaps (spike)

- one editor, no tabs; no go-to-definition, hover docs, formatting
- backend-initiated document changes (`DocPatched`) are not merged yet
- themes and keymaps follow the settings, but there is no Settings UI to switch them yet
- duplicate completion items are not merged
- error squiggles are not visible under Mesa llvmpipe (hover shows the diagnostic);
  unverified on real GPUs
- text sync diffs the whole buffer per edit (O(n)); fine for a spike

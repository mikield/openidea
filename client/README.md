# GPUI client (phase-0 spike)

A native Rust UI (GPUI + gpui-component) for the **unmodified** `openidea-server`.
The client is remote-first: even a local session spawns `openidea-server --stdio`
and speaks the remote-development protocol (`crates/ide-host/src/rpc.rs`), so local
and SSH sessions share one code path.

This is a separate Cargo workspace on purpose: the root `Cargo.toml` / `Cargo.lock`
and everything under `crates/` stay untouched, so upstream merges stay clean.
Upstream crates are used read-only (`ide-core`, `ide-jsonrpc`).

```
client/
├─ crates/client-rpc   typed protocol client, UTF-8 ↔ UTF-16 coordinates, e2e tests
└─ crates/client-app   `openidea-gpui`: project tree, editor, problems, status bar
```

## Run

```sh
cargo build -p ide-host --bin openidea-server          # at the repo root
cd client && cargo build -p client-app
OPENIDEA_SERVER=../target/debug/openidea-server \
  target/debug/openidea-gpui /path/to/project --open src/main.rs
```

Linux needs a Vulkan or GL driver (headless: Xvfb + Mesa llvmpipe works).
`OPENIDEA_SERVER_HOME` isolates the backend's config and caches.

## What works

- project tree (lazy `fs_read_dir`, refreshed on `FsChanged`)
- open / edit / save documents (`doc_open`, `doc_edit`, `doc_save`); edits are
  diffed against the last synced text and sent in UTF-16 offsets
- diagnostics from the server's language servers (problems panel, editor hover)
- completion through `lsp_completion`, filtered and ranked client-side
- one ordered queue per document: completion never runs against text the
  server has not seen, save never overtakes an edit

`cargo test --workspace` runs the end-to-end tests against the real server
(built as above); the rust-analyzer test needs `rust-analyzer` on `PATH`.

## Known gaps (spike)

- one editor, no tabs; no go-to-definition, hover docs, formatting
- backend-initiated document changes (`DocPatched`) are not merged yet
- theme is gpui-component's built-in dark; OpenIDEA themes live in the web
  frontend's CSS tokens and are not shared yet
- duplicate completion items are not merged
- error squiggles are not visible under Mesa llvmpipe (hover shows the diagnostic);
  unverified on real GPUs
- text sync diffs the whole buffer per edit (O(n)); fine for a spike

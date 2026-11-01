# Porting the React UI to GPUI — working notes

The single source of context for anyone (human or agent) continuing the port.
The full plan with diagrams lives in the doc
[OpenIDEA → Rust UI: план переносу](https://claude.ai/artifact/NeT5xt9nEANezbhpnVufkQ);
this file holds the decisions, rules and next steps in short.

Started in cloud session `session_0119LHos6FRaLnRfpPRXyff5` (Nov 1–2, 2026), branch
`spike/gpui-client`, on upstream `l0ng-ai/openidea` @ `4015a5b`.

## Decisions

| Decision | Choice |
| --- | --- |
| Goal | Own product: a faster IDE with a native Rust UI, not a contribution to upstream |
| UI stack | GPUI (`gpui-pre =0.3.10`) + `gpui-component =0.8.1`, pinned exactly |
| Architecture | Plugin-first: a small core with extension points; almost every feature is a plugin |
| Third-party plugins | Later. First-party plugins are Rust crates linked statically; WASM with declarative UI after parity |
| Upstream | Keep syncing `crates/` from `l0ng-ai/openidea`; never edit upstream files |
| Remote dev | Kept. The client always talks to the backend through the protocol, locally too |
| Repository | This fork for now, a separate repository later |
| Wave order | 1 core → 2 editor → 3 VCS + Run → 4 integrations (DB, HTTP, GitHub, Docker, SSH) → 5 debugger + agents → 6 tail + parity |
| OS | macOS, Windows and Linux from wave 1 (CI matrix is part of the wave 1 gate) |

## Ground rules

- **Upstream zone, read-only:** `crates/`, `src/`, `src-tauri/`, root `Cargo.toml` / `Cargo.lock`,
  upstream workflows. Our code lives in `client/` (a separate Cargo workspace), plus
  `AGENTS.md` and `.github/workflows/client.yml`.
- **Missing backend commands** go into our own server binary (wave 1: `client/crates/oi-server`):
  `ide_host::rpc::Dispatcher::standard()` + extra commands via the public `ide_host::ops::*`.
  Never by editing `ide-host`.
- **Client-local services** (DB, HTTP, GitHub, Docker, SSH, WASM plugins) link the upstream
  crates (`ide-db`, `ide-http`, …) into the client process, as Tauri does.
- **Port behaviour, not code.** The React feature and its tests are the spec. Record the upstream
  revision you ported from in the ledger below; after each upstream sync, diff
  `src/features/<feature>` from that revision and port the delta.
- **Every feature is a plugin** from its first commit: a UI crate, plus a backend part only
  when it needs server commands. Types crossing the boundary are serde.

## Waves and gates

| Wave | Scope (React lines of TS) | Gate: done when |
| --- | --- | --- |
| 0 | Spike (done) | e2e tests against the real server, screenshots under Xvfb |
| 1 | Core + plugin model (48k): dock, tool windows, tabs, actions + keymaps, settings, themes, project tree, plugin host, `oi-server` | 3 plugins built only on extension points; layout persists; CI green on 3 OS |
| 2 | Editor + code insight (60k): editor, completion, nav, refactor, search, terminal, vim, templates | Editor gate passed (below); used daily |
| 3 | VCS + Run (44k) | `e2e/diff`, `e2e/gitIgnore` scenarios pass; run and test from the UI |
| 4 | Integrations as plugins (35k): Database, HTTP client, GitHub, Docker, SSH, deployment | Each is its own plugin in the client process, no new protocol commands |
| 5 | Debugger + agents (14k): DAP UI, coverage, profiler | breakpoint → step → variables on Rust and PHP; an agent task in a worktree |
| 6 | Tail + parity (2k): notebook, perf, remaining e2e, packaging | `perf/budgets.json` met; Tauri build switched off |

**Editor gate** (start of wave 2): start with the gpui-component editor (A); if it fails two or
more items, fork the `input` module of `gpui-base` (Apache-2.0) into our crate (B). Zed's editor
is GPL: never copy from it.
- typing p95 ≤ 20 ms, max ≤ 33 ms on a 10k-line file (upstream `perf/budgets.json`)
- a 100k-line file opens, scrolls and edits
- IME (Ukrainian, CJK, dead keys) on all 3 OS
- multi-cursor, column selection, folding, undo kept across backend `DocPatched`
- decoration API: inlay hints, gutter icons, inline diff — without forking
- delta-based sync instead of whole-buffer diff
- diagnostic squiggles visible on a real GPU

## Wave 1: next steps, in order

1. ~~`Transport` trait in `client-rpc`~~ — done: `client_rpc::Transport`, `Stdio` (was `ServerCommand`),
   `Connection::connect(&dyn Transport, …)`; SSH and in-process transports come later.
2. `client/crates/oi-server`: `Dispatcher::standard()` + the commands the next features need;
   the client starts it instead of `openidea-server`.
3. `client-core`: extension registry (`ToolWindowFactory`, `EditorFeature`, `SettingsPage`,
   `Theme`, …), action system, dock with tool windows and tabs, settings storage via
   `settings_get` / `settings_set`.
4. Keymaps and themes as data: convert `src/platform/keymaps/*.ts` (action id → shortcuts) and
   `src/styles/tokens.css` / `scripts/theme/` into JSON with a script; map tokens onto
   gpui-component `ThemeColor` and the highlight theme. Themes become a contribution plugin.
5. Move the spike's project tree into the first plugin; the spike's `workspace.rs` dissolves
   into core + plugins.
6. `client.yml` CI green on macOS, Windows and Linux.

## Gotchas found in the spike

- **Coordinates:** the protocol counts UTF-16 units; the gpui-component editor's `Position`
  counts chars; ropes use UTF-8 bytes. Convert at every boundary (`client_rpc::text`).
- **Edits:** the editor emits `InputEvent::Change` without a delta; the spike diffs the buffer
  (O(n) per keystroke). Edits, completion and save go through one ordered queue per document
  (`doc_sync.rs`), otherwise completion races the edit it depends on.
- **Completion:** language servers leave filtering to the client and the editor does not
  filter, so the client ranks (`completion.rs`). Without a `text_edit` the editor keeps the
  typed prefix. rust-analyzer returns duplicates (`is_empty` twice), not merged yet.
- **Diagnostics:** push them into the editor sorted by range and call `state.refresh(cx)`.
  Squiggles are not visible under Mesa llvmpipe (hover shows the diagnostic); unverified on GPUs.
- **Backend:** language servers start only for trusted projects (`project_trust_set`).
  A stale `baseVersion` in `doc_edit` is refused, so `DocPatched` must be merged (not done).
- **Ecosystem:** many GPUI forks on crates.io; gpui-component pins `gpui-pre`. The Linux
  platform crate is `gpui-pre-platform` (wgpu renderer: Vulkan or GL).

## Dev setup

```sh
cargo build -p ide-host --bin openidea-server     # repo root, once (~20 min cold on 2 cores)
cd client
cargo run -p client-app -- /path/to/project --open src/main.rs
cargo test --workspace                            # e2e tests use target/debug/openidea-server
```

The client finds the server via `$OPENIDEA_SERVER`, next to its own binary, in the repo's
`target/{debug,release}`, then on `PATH`. `OPENIDEA_SERVER_HOME` isolates backend config and
caches. `rust-analyzer` on `PATH` enables the language-server test.

Platform dependencies:
- **Linux** (verified on Ubuntu 24.04): `libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev
  libvulkan-dev libx11-xcb-dev libxcb-xkb-dev libfontconfig-dev` and a Vulkan/GL driver
  (`mesa-vulkan-drivers` for software rendering).
- **macOS, Windows:** not built yet. `runtime_shaders` is enabled, so macOS should not need
  full Xcode for Metal shaders; Windows needs the MSVC toolchain. `client.yml` CI is the check.

Headless check (Linux, agents and CI): `client/scripts/headless-shot.sh PROJECT [FILE] [OUT.png]`.

## Port ledger

| Feature | Plugin crate | Ported from upstream rev | Status |
| --- | --- | --- | --- |
| project-tree | — | 4015a5b (spike only) | spike |
| editor (basic) | — | 4015a5b (spike only) | spike |

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
2. ~~`client/crates/oi-server`~~ — done: `oi_server::dispatcher()` = `Dispatcher::standard()` +
   `commands::register` (only `oi_server_info` so far: a client tells `oi-server` from a plain
   `openidea-server` by it); `Stdio::locate()` finds `oi-server`. Step 3 needs no new commands:
   React keeps tool-window layout in `localStorage`, so layout stays client-side, and settings
   use the standard `settings_get` / `settings_set`. `main.rs` mirrors upstream's
   `crates/ide-host/src/bin/server.rs`: re-check it after each upstream sync.
3. ~~`client-core`~~ — done, modelled on React's `contributions.ts` / `actions.ts`:
   - `Plugin::activate(&mut Extensions)` registers by id: `ToolWindow` (anchor left/right/bottom,
     order), `StatusItem`, `SettingsPage`, the editor area, actions and key bindings. Plugins
     hand in plain views (`ViewBuilder`); the core wraps them as dock panels.
   - Actions: dotted string ids; every binding dispatches one GPUI action `RunAction { id }`,
     so keymaps (step 4) are plain data. An action runs inside the `Workbench` update.
   - Dock: gpui-component's `DockArea` (`DockSkin`): tabs per dock, drag between docks,
     `workbench.toggle{Left,Right,Bottom}Dock`, `Workbench::toggle_tool_window`. The layout is
     saved per project root on the client (`client_core::layout`, `$OPENIDEA_CLIENT_HOME`),
     debounced; a tool window added since the last save joins its anchor's dock.
   - `Backend` (connection + server events as a GPUI emitter) and `SettingsStore`
     (`settings_get` / `settings_set`, re-read on `SettingsChanged`).
   - The spike's `Workspace` is now the first plugin (`spike_plugin.rs`): editor area +
     `file.save` on Ctrl/Cmd+S.
   - Deferred on purpose: `EditorFeature` to wave 2 (it needs the editor it extends), themes to
     step 4; the Settings dialog and a command palette consume `SettingsPage` / actions later.
     One window per process for now: panel builders are a GPUI global.
4. ~~Keymaps and themes as data~~ — done:
   - `node client/scripts/export-presets.ts` (Node >= 23, no packages) writes
     `crates/plugin-presets/data/`: the 6 keymaps imported from `src/platform/keymaps/*.ts`, and
     the 7 built-in themes resolved from `src/styles/tokens.css` (cascade per theme attributes,
     `var()`, `color-mix()`, `hsl()`). Each file records its upstream revision (`from`); CI runs
     it with `--check`, so an upstream change to keymaps or tokens fails the build until re-exported.
   - `plugin-presets` contributes them (`Extensions::add_keymap` / `add_theme`); `client-core`
     picks one of each from the settings (`keymap`, `keymapOverrides`, `customKeymaps`;
     `appearance.theme` / `syncWithOs` / preferred themes / `accent`) and re-applies on
     `SettingsChanged` and OS light/dark changes.
   - Keymap: only ids of registered actions are bound (others keep reaching the focused view);
     double-tapped modifiers (`Shift Shift`) and mouse bindings are not expressible in GPUI yet.
     Tool windows get `toolwindow.<id>` actions (Alt+1 …) as in React. `Extensions::bind` is gone:
     default keys live in keymaps, never on the action.
   - Theme: `client_core::theme` maps tokens onto gpui-component colours and the editor
     highlight style (tables `COLORS` / `EDITOR` / `SYNTAX`).
   - Not yet: the Settings UI to switch them, plugin themes (`data-plugin-theme`), editor
     schemes beyond the theme's own syntax tokens.
5. ~~Project tree into the first plugin~~ — done: the spike's `workspace.rs` is gone.
   - Plugins (each its own crate, on `client-core` only, unaware of each other):
     `plugin-editor` (editor area, doc sync, completion, `file.save`), `plugin-project-tree`
     (`project` tool window, left), `plugin-problems` (`problems` tool window, bottom),
     `plugin-presets`. `client-app` is `main.rs` and the plugin list.
   - New shared services in `Services`: `Editors` (React `EditorService`: open a file, which
     one is active) and `DiagnosticsStore` (each producer's set per file). The backend and
     task status bar items are the core's own `StatusItem`s.
   - The wave 1 gate's "3 plugins built only on extension points" holds (editor, project
     tree, problems); a saved layout without them gets them added to their docks.
6. `client.yml` CI green on macOS, Windows and Linux. (Postponed by the owner: parity first.)
7. ~~The window looks like the React IDE~~ — done (compared against the running React app):
   - Islands skin (`client_core::skin`, our own `DockAreaRenderer` / `TabGroupRenderer` over
     gpui-base): rounded islands on `bg-frame`, `--island-gap`, a bold tool window header
     (tabs when an island holds several), no divider lines.
   - Tool window stripes: left (left tool windows on top, bottom ones below), right; icon
     buttons with tooltips; `ToolWindow.icon` uses React's icon names.
   - Frameless title bar: project avatar (React's `projectColorIndex` / `projectInitials`) and
     name, `TitleItem` contributions (`plugin-vcs`: the branch via `vcs_branch_info`), window
     controls.
   - Icons: upstream's expui / openidea SVGs served from `src/ui/` (`rust-embed`, read-only),
     the name and file-type tables exported from `src/ui/icons.tsx`; `client_core::Icon`
     draws monochrome icons in the text colour, coloured ones in their light / `_dark` variant.
   - Project tree: root node (module icon, name, path), chevrons, file-type icons, React row
     metrics; editor: a tab with the file icon; empty editor: React's hints with this keymap's
     keys (`keymap::shortcut_label`).
   - Editor tabs (`plugin-editor`): several documents, IDEA's "activate left" on close,
     `editor.closeTab` / `editor.nextTab` / `editor.prevTab`, middle-click close, the dirty dot.
   - Terminal (`plugin-terminal`, Alt+F12): `alacritty_terminal` (Apache-2.0) parses the PTY
     output; our own GPUI grid in the theme's ANSI colours; keys encoded to VT sequences,
     bracketed paste (Ctrl+Shift+V), scrollback; `term_spawn` / `term_write` / `term_resize` in
     one ordered queue; the terminal's replies to queries (conpty's DSR) go back to the PTY.
     Tabs "Local", "Local (2)"…; an end-to-end test runs a real shell on `oi-server`.
     Not yet: selection / copy, mouse reporting, shell choice, terminal settings.
   - Search Everywhere (`plugin-search`): Shift, Shift and Go to File / Class / Symbol / Action /
     Recent Files open one popup on the matching tab (React's `sectionsFor`, limits, headers in
     All); `find_files` / `find_symbols` / `recent_files` plus the registered actions; the editor
     marks opened files recent. Popups are a core service (`Workbench::show_overlay`, closed by
     `DismissEvent` or a click outside). Double-tapped modifiers are recognised by the core
     (`keymap::DoubleTap`, React's 300 ms) and run the keymap's `Shift Shift`-style bindings.
     Not yet: the Text tab, a symbol jumps to its file but not its line, recent actions.
   - Folder roles in the project tree from `project_roots` (source / test / resources / test
     resources / excluded, `.github`); JVM packages and build output folders come with wave 2.
   - `--run-action ID` runs an action once the window is up (screenshots without input).
   - IDEA's tool window handling (the skin draws it; base only offers callbacks): the inner edge of
     each dock resizes it (`DockContext::resize_to`); the header's options menu moves a tool
     window (Move to Left / Right / Bottom) and Hide closes its dock; the header drags it onto
     another island (`drag_panel` / `drop_panel`, a drop placeholder); stripe buttons drag to
     another stripe; stripes follow where each tool window docks now. One tool window shows per
     island (no tab strip), as in IDEA. `ToolWindow.own_header` lets a view draw its header row
     (Terminal: title, session tabs, `tool_window_controls`).
   - Type and chrome as React: 13 / 12 px (`--fs-ui` / `--fs-small`), Inter when installed (else
     the system UI font; the fonts React ships come from npm and are not in the repository),
     the editor font from settings else JetBrains Mono, 40 px title bar, an empty status bar
     when idle.
   - Still missing against React: main menu, run configuration and search / settings buttons in
     the title bar, right status items (Vim, interpreter, memory), editor tab overflow / pinning /
     splits, terminal selection.
   - Reference for IDEA behaviour where the React port is silent: OpenIDE
     (gitflic.ru/project/openide/openide, an IntelliJ Community fork, Apache-2.0); the React
     code stays the spec.

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
- **Trust gate:** every `invoke`, ours included, goes through upstream's `authorize_command`:
  outside its prefix allowlist (`fs_`, `doc_`, `settings_`, … in `ide-host/src/trust.rs`) a
  command needs a trusted project open in the window, so `oi_*` commands do too.
- **GPUI tests:** `#[gpui::test]` with `TestAppContext::add_window_view` works headless on every
  OS (`gpui` dev-dependency with `test-support`). Anything awaiting the tokio-side connection
  needs `cx.executor().allow_parking()`, else the test scheduler reports non-determinism.
- **Dock renderer types:** `ResizeHandleContext` is not re-exported by gpui-component;
  `gpui-base` is a direct dependency pinned with it (`=0.8.1`).
- **gpui-component highlight keys:** built with `syntect` (no `tree-sitter`), it compiles
  `highlighter/wasm_stub.rs`, where the editor colours are `editor_background`…; with
  `tree-sitter` they are `editor.background`…. `theme_config` writes both spellings.
- **Settings file:** `oi-server` emits `SettingsChanged` only for `settings_set`; editing
  `config/settings.toml` by hand takes effect on the next start.
- **Windows paths:** `std::fs::canonicalize` returns `\\?\C:\…`, the server reports `C:/…`;
  strip the verbatim prefix before comparing (or use `dunce`).
- **Backend:** language servers start only for trusted projects (`project_trust_set`).
  A stale `baseVersion` in `doc_edit` is refused, so `DocPatched` must be merged (not done).
- **Ecosystem:** many GPUI forks on crates.io; gpui-component pins `gpui-pre`. The Linux
  platform crate is `gpui-pre-platform` (wgpu renderer: Vulkan or GL).

## Dev setup

```sh
cd client
cargo build -p oi-server                          # the backend (~20 min cold on 2 cores)
cargo run -p client-app -- /path/to/project --open src/main.rs
cargo test --workspace                            # e2e tests use target/debug/oi-server
```

The client finds the server via `$OPENIDEA_SERVER`, `oi-server` next to its own binary, in
`client/target/{debug,release}`, then on `PATH`. `OPENIDEA_SERVER_HOME` isolates backend config and
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
| project-tree | plugin-project-tree | 4015a5b (spike behaviour, not yet `src/features/project`) | basic |
| problems | plugin-problems | 4015a5b (spike behaviour) | basic |
| editor (basic) | plugin-editor | 4015a5b (spike behaviour) | tabs, no splits |
| terminal | plugin-terminal | 4015a5b (`src/features/terminal`, behaviour) | basic: no selection, no settings |
| keymaps (6 presets) | plugin-presets | 87bf89d (`src/platform/keymaps`) | data, re-export with the script |
| themes (7 built-in) | plugin-presets | 2c9a647 (`tokens.css`, `themes.ts`) | data, re-export with the script |

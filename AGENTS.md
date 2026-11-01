# Agent instructions (fork: mikield/openidea)

This fork ports OpenIDEA's React/Tauri UI to a native Rust UI (GPUI) with a plugin-first core,
while keeping the upstream backend (`l0ng-ai/openidea`) mergeable.

Read `client/PORTING.md` before any work on the port: decisions, rules, wave plan, next steps,
gotchas, dev setup and the port ledger. `client/README.md` describes what exists today.

Hard rules:
- Never edit upstream files: `crates/`, `src/`, `src-tauri/`, root `Cargo.toml` / `Cargo.lock`,
  upstream workflows. New code goes into `client/` (its own Cargo workspace).
- Backend commands the client needs are added in our own server binary through the public
  `ide_host::ops` / `ide_host::rpc::Dispatcher`, not by changing `ide-host`.
- Every ported feature is a plugin on core extension points from its first commit, and gets a row
  in the port ledger with the upstream revision it was ported from.
- Before committing in `client/`: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace`.

The owner communicates in Ukrainian; keep technical terms in English.

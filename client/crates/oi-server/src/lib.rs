//! `oi-server`: the backend the GPUI client starts, locally over stdio and later over SSH.
//!
//! It is upstream's `openidea-server` with our own commands on top: the
//! [`Dispatcher::standard`] set from `ide-host`, plus [`commands::register`].
//! Upstream crates are never edited; anything the client needs from the
//! backend is added here through the public `ide_host::ops` / `ide_host::rpc`.

pub mod commands;

use ide_host::rpc::Dispatcher;

/// Upstream's standard commands plus ours.
pub fn dispatcher() -> Dispatcher {
    let mut d = Dispatcher::standard();
    commands::register(&mut d);
    d
}

#[cfg(test)]
mod tests {
    #[test]
    fn serves_upstream_and_own_commands() {
        let commands = super::dispatcher().commands();
        for c in ["open_project", "doc_open", "settings_get", "settings_set", super::commands::SERVER_INFO] {
            assert!(commands.contains(&c), "missing {c}");
        }
    }
}

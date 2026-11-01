//! How a [`Connection`](crate::Connection) reaches its backend.
//!
//! A transport only opens the byte stream and wraps it in a JSON-RPC [`Peer`];
//! `initialize`, events and commands are the same over every transport. Today
//! there is [`Stdio`]; SSH and an in-process server come later.

use ide_core::{IdeError, IdeResult};
use ide_jsonrpc::{Handler, Peer};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A way to open a JSON-RPC connection to a server (`oi-server`, `openidea-server`).
pub trait Transport: Send + Sync {
    /// Opens the connection; messages from the server go to `handler`.
    ///
    /// Called from within a Tokio runtime, which then drives the connection.
    fn connect(&self, handler: Arc<dyn Handler>) -> IdeResult<Peer>;

    /// What this transport connects to, for logs and errors (a binary, a host).
    fn describe(&self) -> String;
}

/// Spawns a server (`oi-server --stdio`) locally and talks over its stdin/stdout.
#[derive(Debug, Clone)]
pub struct Stdio {
    /// The server binary: `oi-server`, or any `openidea-server` that speaks the protocol.
    pub program: PathBuf,
    /// `OPENIDEA_SERVER_HOME`: config, data and caches of this backend.
    pub home: Option<PathBuf>,
}

impl Stdio {
    /// The server from `$OPENIDEA_SERVER`, else `oi-server` next to the current executable,
    /// else in the client workspace's own build (`client/target/{debug,release}`, so tests and
    /// `cargo run` find it during development), else on `PATH`.
    pub fn locate() -> Self {
        let name = format!("oi-server{}", std::env::consts::EXE_SUFFIX);
        let program = std::env::var_os("OPENIDEA_SERVER").map(PathBuf::from).unwrap_or_else(|| {
            let sibling = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join(&name)));
            let client_target = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
            let dev_builds = ["debug", "release"].map(|profile| client_target.join(profile).join(&name));
            sibling.into_iter().chain(dev_builds).find(|p| p.exists()).unwrap_or_else(|| PathBuf::from(&name))
        });
        Self { program, home: std::env::var_os("OPENIDEA_SERVER_HOME").map(PathBuf::from) }
    }
}

impl Transport for Stdio {
    fn connect(&self, handler: Arc<dyn Handler>) -> IdeResult<Peer> {
        let env: Vec<(String, String)> =
            self.home.iter().map(|h| ("OPENIDEA_SERVER_HOME".to_owned(), h.to_string_lossy().into_owned())).collect();
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
        Peer::spawn(
            &self.program.to_string_lossy(),
            &["--stdio".to_owned()],
            &cwd,
            &env,
            handler,
            Box::new(|line| tracing::debug!(target: "oi-server", "{line}")),
        )
        .map_err(|e| IdeError::internal(format!("cannot start {}: {e}", self.program.display())))
    }

    fn describe(&self) -> String {
        self.program.display().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Connection;
    use ide_jsonrpc::codes;
    use parking_lot::Mutex;
    use serde_json::{Value, json};

    /// An in-memory server that answers `initialize` with `protocol` and `commands` with one command.
    struct FakeServer {
        protocol: u32,
        peer: Mutex<Option<Peer>>,
    }

    impl Handler for FakeServer {
        fn on_notification(&self, _: &str, _: Value) {}

        fn on_request(&self, method: &str, _: Value) -> Result<Value, (i64, String)> {
            match method {
                "initialize" => Ok(json!({
                    "version": "test", "protocol": self.protocol, "os": "test", "arch": "test", "home": "/",
                })),
                "commands" => Ok(json!(["open_project"])),
                other => Err((codes::METHOD_NOT_FOUND, other.to_owned())),
            }
        }
    }

    /// Serves a [`FakeServer`] over in-memory pipes, as an in-process transport would.
    struct InMemory(Arc<FakeServer>);

    impl Transport for InMemory {
        fn connect(&self, handler: Arc<dyn Handler>) -> IdeResult<Peer> {
            let (client, server) = tokio::io::duplex(64 * 1024);
            let (server_read, server_write) = tokio::io::split(server);
            *self.0.peer.lock() = Some(Peer::connect(server_read, server_write, self.0.clone()));
            let (client_read, client_write) = tokio::io::split(client);
            Ok(Peer::connect(client_read, client_write, handler))
        }

        fn describe(&self) -> String {
            "in-memory".to_owned()
        }
    }

    fn in_memory(protocol: u32) -> InMemory {
        InMemory(Arc::new(FakeServer { protocol, peer: Mutex::new(None) }))
    }

    #[tokio::test]
    async fn connection_runs_over_any_transport() {
        let transport = in_memory(crate::proto::PROTOCOL_VERSION);
        let (conn, info, _events) = Connection::connect(&transport, "main").await.unwrap();
        assert_eq!(info.protocol, crate::proto::PROTOCOL_VERSION);
        assert_eq!(conn.commands().await.unwrap(), ["open_project"]);
    }

    #[tokio::test]
    async fn protocol_mismatch_is_refused() {
        let transport = in_memory(crate::proto::PROTOCOL_VERSION + 1);
        let err = Connection::connect(&transport, "main").await.err().expect("mismatch accepted");
        assert!(err.message.contains("protocol"), "{}", err.message);
    }
}

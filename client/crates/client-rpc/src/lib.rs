//! Typed client of the `openidea-server` protocol (see `ide_host::rpc`).
//!
//! The client is remote-first: even a local session spawns `openidea-server
//! --stdio` and talks JSON-RPC to it, so local and SSH sessions share one path.
//!
//! Every call runs on the connection's own Tokio runtime and returns a
//! runtime-agnostic future, so UI executors (GPUI's) can await it directly.

pub mod proto;
pub mod text;

use ide_core::{ChangeSet, DocId, IdeError, IdeEvent, IdeResult, Pos, RequestId};
use ide_jsonrpc::{Handler, Peer, RpcError, codes};
use parking_lot::Mutex;
use proto::{ChannelMessage, CompletionList, DocumentInfo, EventMessage, FileEntry, Invoke, ProjectInfo, ServerInfo};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::runtime::Handle;
use tokio::sync::mpsc;

/// How to start the backend.
#[derive(Debug, Clone)]
pub struct ServerCommand {
    /// The `openidea-server` binary.
    pub program: PathBuf,
    /// `OPENIDEA_SERVER_HOME`: config, data and caches of this backend.
    pub home: Option<PathBuf>,
}

impl ServerCommand {
    /// `openidea-server` from `$OPENIDEA_SERVER`, else next to the current executable,
    /// else on `PATH`.
    pub fn locate() -> Self {
        let program = std::env::var_os("OPENIDEA_SERVER").map(PathBuf::from).unwrap_or_else(|| {
            let sibling = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("openidea-server")));
            sibling.filter(|p| p.exists()).unwrap_or_else(|| PathBuf::from("openidea-server"))
        });
        Self { program, home: std::env::var_os("OPENIDEA_SERVER_HOME").map(PathBuf::from) }
    }
}

/// One project window's connection to a backend.
pub struct Connection {
    peer: Peer,
    window: String,
    rt: Handle,
    channels: Channels,
}

type Channels = Arc<Mutex<HashMap<u64, mpsc::UnboundedSender<Value>>>>;

impl Connection {
    /// Spawns the backend over stdio and performs `initialize`.
    ///
    /// Events for `window` (and broadcast ones) arrive on the returned receiver.
    /// Must be called from within a Tokio runtime; calls then run on that runtime.
    pub async fn spawn(
        server: &ServerCommand,
        window: impl Into<String>,
    ) -> IdeResult<(Arc<Self>, ServerInfo, mpsc::UnboundedReceiver<IdeEvent>)> {
        let window = window.into();
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let channels: Channels = Arc::default();
        let handler = Arc::new(ClientHandler { window: window.clone(), events: events_tx, channels: channels.clone() });
        let env: Vec<(String, String)> =
            server.home.iter().map(|h| ("OPENIDEA_SERVER_HOME".to_owned(), h.to_string_lossy().into_owned())).collect();
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
        let peer = Peer::spawn(
            &server.program.to_string_lossy(),
            &["--stdio".to_owned()],
            &cwd,
            &env,
            handler,
            Box::new(|line| tracing::debug!(target: "openidea-server", "{line}")),
        )
        .map_err(|e| IdeError::internal(format!("cannot start {}: {e}", server.program.display())))?;
        let conn = Arc::new(Self { peer, window, rt: Handle::current(), channels });
        let info: ServerInfo = conn
            .peer
            .request("initialize", json!({ "version": proto::PROTOCOL_VERSION }), None)
            .await
            .map_err(IdeError::from)?;
        if info.protocol != proto::PROTOCOL_VERSION {
            return Err(IdeError::internal(format!(
                "server speaks protocol {}, client {}",
                info.protocol,
                proto::PROTOCOL_VERSION
            )));
        }
        Ok((conn, info, events_rx))
    }

    pub fn window(&self) -> &str {
        &self.window
    }

    /// Runs `f` on the connection's runtime; the returned future is executor-agnostic.
    fn on_rt<T: Send + 'static>(
        &self,
        f: impl Future<Output = IdeResult<T>> + Send + 'static,
    ) -> impl Future<Output = IdeResult<T>> + Send + 'static {
        let task = self.rt.spawn(f);
        async move { task.await.map_err(|e| IdeError::internal(e.to_string()))? }
    }

    /// `invoke` of desktop command `cmd` with camelCase `args`.
    pub fn invoke<T: DeserializeOwned + Send + 'static>(
        &self,
        cmd: &'static str,
        args: Value,
    ) -> impl Future<Output = IdeResult<T>> + Send + 'static {
        let peer = self.peer.clone();
        let window = self.window.clone();
        self.on_rt(async move {
            peer.request::<_, T>("invoke", Invoke { window: &window, cmd, args }, None).await.map_err(error_from_reply)
        })
    }

    /// Opens a streaming argument: pass the returned `Value` as the argument, read values from the receiver.
    pub fn open_channel(&self) -> (Value, mpsc::UnboundedReceiver<Value>) {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let (tx, rx) = mpsc::unbounded_channel();
        self.channels.lock().insert(id, tx);
        (json!({ "$channel": id }), rx)
    }

    pub fn commands(&self) -> impl Future<Output = IdeResult<Vec<String>>> + Send + 'static {
        let peer = self.peer.clone();
        self.on_rt(async move { peer.request("commands", Value::Null, None).await.map_err(IdeError::from) })
    }

    // ── project ────────────────────────────────────────────────────────────

    pub fn open_project(&self, path: &Path) -> impl Future<Output = IdeResult<ProjectInfo>> + Send + 'static {
        self.invoke("open_project", json!({ "path": path }))
    }

    /// Marks `path` trusted (language servers, build tools and runs need it).
    pub fn trust_project(&self, path: &Path) -> impl Future<Output = IdeResult<()>> + Send + 'static {
        self.invoke("project_trust_set", json!({ "path": path, "trusted": true, "folder": true }))
    }

    pub fn read_dir(&self, path: &Path) -> impl Future<Output = IdeResult<Vec<FileEntry>>> + Send + 'static {
        self.invoke("fs_read_dir", json!({ "path": path }))
    }

    // ── documents ──────────────────────────────────────────────────────────

    pub fn doc_open(&self, path: &Path) -> impl Future<Output = IdeResult<DocumentInfo>> + Send + 'static {
        self.invoke("doc_open", json!({ "path": path }))
    }

    pub fn doc_close(&self, doc: DocId) -> impl Future<Output = IdeResult<()>> + Send + 'static {
        self.invoke("doc_close", json!({ "doc": doc }))
    }

    /// Applies `changes` on top of `base_version`; the new version.
    pub fn doc_edit(
        &self,
        doc: DocId,
        base_version: u32,
        changes: ChangeSet,
    ) -> impl Future<Output = IdeResult<u32>> + Send + 'static {
        self.invoke("doc_edit", json!({ "doc": doc, "baseVersion": base_version, "changes": changes }))
    }

    pub fn doc_save(&self, doc: DocId) -> impl Future<Output = IdeResult<()>> + Send + 'static {
        self.invoke("doc_save", json!({ "doc": doc }))
    }

    // ── code insight ───────────────────────────────────────────────────────

    pub fn completion(
        &self,
        doc: DocId,
        pos: Pos,
        trigger: Option<String>,
    ) -> impl Future<Output = IdeResult<CompletionList>> + Send + 'static {
        self.invoke(
            "lsp_completion",
            json!({ "request": RequestId::next(), "doc": doc, "pos": pos, "trigger": trigger }),
        )
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.peer.kill();
    }
}

struct ClientHandler {
    window: String,
    events: mpsc::UnboundedSender<IdeEvent>,
    channels: Channels,
}

impl Handler for ClientHandler {
    fn on_notification(&self, method: &str, params: Value) {
        match method {
            "event" => match serde_json::from_value::<EventMessage>(params) {
                Ok(m) if m.window.as_deref().is_none_or(|w| w == self.window) => {
                    let _ = self.events.send(m.event);
                }
                Ok(_) => {}
                Err(e) => tracing::warn!("bad event: {e}"),
            },
            "channel" => match serde_json::from_value::<ChannelMessage>(params) {
                Ok(m) => {
                    let mut channels = self.channels.lock();
                    if let Some(tx) = channels.get(&m.id)
                        && tx.send(m.data).is_err()
                    {
                        channels.remove(&m.id);
                    }
                }
                Err(e) => tracing::warn!("bad channel message: {e}"),
            },
            other => tracing::debug!("unhandled notification {other}"),
        }
    }

    fn on_request(&self, method: &str, _params: Value) -> Result<Value, (i64, String)> {
        Err((codes::METHOD_NOT_FOUND, format!("client handles no requests ({method})")))
    }
}

/// The `IdeError` of an `invoke` reply: the server sends the error's JSON as the message.
fn error_from_reply(e: RpcError) -> IdeError {
    match e {
        RpcError::Server { message, .. } => serde_json::from_str(&message)
            .unwrap_or_else(|_| IdeError::new(ide_core::error::ErrorKind::Protocol, message)),
        other => other.into(),
    }
}

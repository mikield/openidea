//! Keeps one open document in sync with the backend.
//!
//! Edits, completion and save go through one ordered queue, so a completion
//! request is never answered against text the server has not seen yet, and a
//! save never overtakes the edit before it.

use client_rpc::proto::{CompletionList, DocumentInfo};
use client_rpc::{Connection, text};
use ide_core::{ChangeSet, DocId, IdeError, IdeResult};
use std::sync::Arc;
use tokio::runtime::Handle;
use tokio::sync::{mpsc, oneshot};

enum Op {
    /// The editor's full text after a change.
    Edit(String),
    Complete {
        text: String,
        offset: usize,
        trigger: Option<String>,
        reply: oneshot::Sender<IdeResult<CompletionList>>,
    },
    Save(oneshot::Sender<IdeResult<()>>),
}

#[derive(Clone)]
pub struct DocSync {
    pub doc: DocId,
    tx: mpsc::UnboundedSender<Op>,
}

impl DocSync {
    pub fn start(conn: Arc<Connection>, rt: &Handle, info: &DocumentInfo) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        rt.spawn(run(conn, info.id, info.text.clone(), info.version, rx));
        Self { doc: info.id, tx }
    }

    pub fn edit(&self, text: String) {
        let _ = self.tx.send(Op::Edit(text));
    }

    /// Completion at byte `offset` of `text` (the editor's current text).
    pub fn complete(
        &self,
        text: String,
        offset: usize,
        trigger: Option<String>,
    ) -> impl Future<Output = IdeResult<CompletionList>> + Send + 'static {
        let (reply, rx) = oneshot::channel();
        let sent = self.tx.send(Op::Complete { text, offset, trigger, reply });
        async move {
            sent.map_err(|_| IdeError::internal("document closed"))?;
            rx.await.map_err(|_| IdeError::internal("document closed"))?
        }
    }

    pub fn save(&self) -> impl Future<Output = IdeResult<()>> + Send + 'static {
        let (reply, rx) = oneshot::channel();
        let sent = self.tx.send(Op::Save(reply));
        async move {
            sent.map_err(|_| IdeError::internal("document closed"))?;
            rx.await.map_err(|_| IdeError::internal("document closed"))?
        }
    }
}

async fn run(
    conn: Arc<Connection>,
    doc: DocId,
    mut synced: String,
    mut version: u32,
    mut rx: mpsc::UnboundedReceiver<Op>,
) {
    while let Some(op) = rx.recv().await {
        match op {
            Op::Edit(new) => {
                push(&conn, doc, &mut synced, &mut version, new).await;
            }
            Op::Complete { text, offset, trigger, reply } => {
                push(&conn, doc, &mut synced, &mut version, text).await;
                let pos = text::byte_to_pos(&synced, offset);
                let _ = reply.send(conn.completion(doc, pos, trigger).await);
            }
            Op::Save(reply) => {
                let _ = reply.send(conn.doc_save(doc).await);
            }
        }
    }
    let _ = conn.doc_close(doc).await;
}

async fn push(conn: &Connection, doc: DocId, synced: &mut String, version: &mut u32, new: String) {
    let Some(change) = text::diff(synced, &new) else { return };
    match conn.doc_edit(doc, *version, ChangeSet { changes: vec![change] }).await {
        Ok(v) => {
            *version = v;
            *synced = new;
        }
        // Out of sync (the backend changed the document meanwhile): the spike does
        // not merge `DocPatched` yet, so it reports instead of guessing.
        Err(e) => tracing::warn!("doc_edit {doc}: {e}"),
    }
}

//! Keeps one open document in sync with the backend.
//!
//! Edits, completion and save go through one ordered queue, so a completion
//! request is never answered against text the server has not seen yet, and a
//! save never overtakes the edit before it.

use client_rpc::proto::{CompletionList, DocumentInfo};
use client_rpc::{Connection, text};
use ide_core::{ChangeSet, DocId, IdeError, IdeResult, Pos, TextChange};
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
    /// Sync `text`, then answer with the protocol position of byte `offset` in it.
    At {
        text: String,
        offset: usize,
        reply: oneshot::Sender<Pos>,
    },
    /// The backend changed the document (`DocPatched`): adopt its changes.
    Patched {
        base_version: u32,
        version: u32,
        changes: Vec<TextChange>,
    },
}

#[derive(Clone)]
pub struct DocSync {
    pub doc: DocId,
    conn: Arc<Connection>,
    tx: mpsc::UnboundedSender<Op>,
}

impl DocSync {
    pub fn start(conn: Arc<Connection>, rt: &Handle, info: &DocumentInfo) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        rt.spawn(run(conn.clone(), info.id, info.text.clone(), info.version, rx));
        Self { doc: info.id, conn, tx }
    }

    pub fn connection(&self) -> &Arc<Connection> {
        &self.conn
    }

    /// The protocol position of byte `offset` in `text` (the editor's current text), once
    /// the backend has that text: for requests about the caret (go to, hover).
    pub fn position(&self, text: String, offset: usize) -> impl Future<Output = IdeResult<Pos>> + Send + 'static {
        let (reply, rx) = oneshot::channel();
        let sent = self.tx.send(Op::At { text, offset, reply });
        async move {
            sent.map_err(|_| IdeError::internal("document closed"))?;
            rx.await.map_err(|_| IdeError::internal("document closed"))
        }
    }

    /// The backend changed the document; the editor applies the same changes.
    pub fn patched(&self, base_version: u32, version: u32, changes: Vec<TextChange>) {
        let _ = self.tx.send(Op::Patched { base_version, version, changes });
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
            Op::At { text, offset, reply } => {
                push(&conn, doc, &mut synced, &mut version, text).await;
                let _ = reply.send(text::byte_to_pos(&synced, offset));
            }
            Op::Patched { base_version, version: next, changes } => {
                if base_version == version {
                    text::apply(&mut synced, &changes);
                    version = next;
                } else {
                    // Our edit and the backend's crossed: the next edit is refused as stale
                    // and reported (see `push`); rebasing is not implemented yet.
                    tracing::warn!("doc {doc}: patch on v{base_version}, have v{version}");
                }
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
        // Out of sync: the backend changed the document while this edit was on its way
        // (`DocPatched` on an older version); reported instead of guessed.
        Err(e) => tracing::warn!("doc_edit {doc}: {e}"),
    }
}

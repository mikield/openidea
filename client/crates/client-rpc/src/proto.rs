//! Wire DTOs of the `openidea-server` protocol that live outside `ide-core`
//! (in `ide-host`, `ide-vfs`, `ide-workspace`, `ide-lsp`).
//!
//! Mirrored with only the fields the client reads: serde ignores the rest, so
//! the client does not link the backend crates and upstream can add fields
//! without breaking it. Once the protocol gets its own crate these go away.

use ide_core::{IdeEvent, IpcPath, Range};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Protocol version this client speaks (`ide_host::rpc::PROTOCOL_VERSION`).
pub const PROTOCOL_VERSION: u32 = 2;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerInfo {
    pub version: String,
    pub protocol: u32,
    pub os: String,
    pub arch: String,
    pub home: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct Invoke<'a> {
    pub window: &'a str,
    pub cmd: &'a str,
    pub args: Value,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct EventMessage {
    pub window: Option<String>,
    pub event: IdeEvent,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ChannelMessage {
    pub id: u64,
    pub data: Value,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectInfo {
    pub trusted: bool,
    pub root: IpcPath,
    pub name: String,
    pub is_git: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EntryKind {
    File,
    Dir,
    Symlink,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    pub name: String,
    pub path: IpcPath,
    pub kind: EntryKind,
    pub ignored: bool,
    pub excluded: bool,
    pub hidden: bool,
    #[serde(default)]
    pub target_kind: Option<EntryKind>,
}

impl FileEntry {
    pub fn is_dir(&self) -> bool {
        self.kind == EntryKind::Dir || (self.kind == EntryKind::Symlink && self.target_kind == Some(EntryKind::Dir))
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentInfo {
    pub id: ide_core::DocId,
    pub path: IpcPath,
    pub text: String,
    pub version: u32,
    pub language: String,
    pub read_only: bool,
    pub dirty: bool,
    #[serde(default)]
    pub large: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionList {
    pub is_incomplete: bool,
    pub items: Vec<CompletionItem>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionItem {
    pub label: String,
    #[serde(default)]
    pub label_detail: Option<String>,
    #[serde(default)]
    pub detail: Option<String>,
    /// `ide_lsp::CompletionKind` in camelCase (`method`, `struct`, …).
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub sort_text: Option<String>,
    #[serde(default)]
    pub filter_text: Option<String>,
    pub insert_text: String,
    pub is_snippet: bool,
    #[serde(default)]
    pub replace_range: Option<Range>,
}

//! Go to definition and hover, backed by the server's language servers
//! (`lsp_goto`, `lsp_hover`; React: `src/editor/lsp/actions.ts`, `gotoTargets`).
//!
//! The editor asks with a byte offset in its text; the document is synced first,
//! so the server answers about the text on screen. Targets travel through the
//! editor as `LocationLink`s in protocol coordinates (UTF-16 columns) and come
//! back through the editor's `show_document` hook, which opens them through
//! [`client_core::Editors`] — the same path for this file and any other.

use crate::doc_sync::DocSync;
use anyhow::anyhow;
use gpui::{App, Task, Window};
use gpui_component::Rope;
use gpui_component::input::{DefinitionProvider, HoverProvider};
use ide_core::{Pos, RequestId};
use serde::Deserialize;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::str::FromStr;

pub struct ServerNavigation {
    pub sync: DocSync,
}

#[derive(Deserialize)]
struct Location {
    path: PathBuf,
    range: ide_core::Range,
}

#[derive(Deserialize)]
struct Markup {
    kind: String,
    value: String,
}

#[derive(Deserialize)]
struct Hover {
    contents: Markup,
}

fn lsp_pos(pos: Pos) -> lsp_types::Position {
    lsp_types::Position::new(pos.line, pos.col)
}

fn lsp_range(range: ide_core::Range) -> lsp_types::Range {
    lsp_types::Range::new(lsp_pos(range.start), lsp_pos(range.end))
}

impl DefinitionProvider for ServerNavigation {
    fn definitions(
        &self,
        text: &Rope,
        offset: usize,
        _: &mut Window,
        cx: &mut App,
    ) -> Task<anyhow::Result<Vec<lsp_types::LocationLink>>> {
        let pos = self.sync.position(text.to_string(), offset);
        let (conn, doc) = (self.sync.connection().clone(), self.sync.doc);
        cx.background_executor().spawn(async move {
            let pos = pos.await.map_err(|e| anyhow!(e.message))?;
            let targets: Vec<Location> = conn
                .invoke("lsp_goto", json!({ "doc": doc, "pos": pos, "kind": "definition" }))
                .await
                .map_err(|e| anyhow!(e.message))?;
            Ok(targets
                .into_iter()
                .filter_map(|t| {
                    Some(lsp_types::LocationLink {
                        origin_selection_range: None,
                        target_uri: path_to_uri(&t.path)?,
                        target_range: lsp_range(t.range),
                        target_selection_range: lsp_range(t.range),
                    })
                })
                .collect())
        })
    }
}

impl HoverProvider for ServerNavigation {
    fn hover(
        &self,
        text: &Rope,
        offset: usize,
        _: &mut Window,
        cx: &mut App,
    ) -> Task<anyhow::Result<Option<lsp_types::Hover>>> {
        let pos = self.sync.position(text.to_string(), offset);
        let (conn, doc) = (self.sync.connection().clone(), self.sync.doc);
        cx.background_executor().spawn(async move {
            let pos = pos.await.map_err(|e| anyhow!(e.message))?;
            let hover: Option<Hover> = conn
                .invoke("lsp_hover", json!({ "request": RequestId::next(), "doc": doc, "pos": pos }))
                .await
                .map_err(|e| anyhow!(e.message))?;
            Ok(hover.filter(|h| !h.contents.value.trim().is_empty()).map(|h| lsp_types::Hover {
                contents: lsp_types::HoverContents::Markup(lsp_types::MarkupContent {
                    kind: if h.contents.kind == "markdown" {
                        lsp_types::MarkupKind::Markdown
                    } else {
                        lsp_types::MarkupKind::PlainText
                    },
                    value: h.contents.value,
                }),
                range: None,
            }))
        })
    }
}

/// `C:\a b\x.rs` → `file:///C:/a%20b/x.rs`; `/a/x.rs` → `file:///a/x.rs`.
pub fn path_to_uri(path: &Path) -> Option<lsp_types::Uri> {
    let slashed = path.to_string_lossy().replace('\\', "/");
    let rooted = if slashed.starts_with('/') { slashed } else { format!("/{slashed}") };
    let mut encoded = String::new();
    for c in rooted.chars() {
        if c.is_ascii_alphanumeric() || "/-_.~:".contains(c) {
            encoded.push(c);
        } else {
            let mut buf = [0u8; 4];
            for b in c.encode_utf8(&mut buf).bytes() {
                encoded.push_str(&format!("%{b:02X}"));
            }
        }
    }
    lsp_types::Uri::from_str(&format!("file://{encoded}")).ok()
}

/// The inverse of [`path_to_uri`]; `None` for other schemes (`jdt://`…, `https://`).
pub fn uri_to_path(uri: &lsp_types::Uri) -> Option<PathBuf> {
    let rest = uri.as_str().strip_prefix("file://")?;
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(b) = rest.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(b);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    let decoded = String::from_utf8(out).ok()?;
    // `/C:/x` is `C:/x` on Windows.
    let windows_drive = decoded.len() > 2 && decoded.as_bytes()[2] == b':' && decoded.starts_with('/');
    Some(PathBuf::from(if windows_drive { &decoded[1..] } else { &decoded }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_round_trip_through_file_uris() {
        for path in ["C:/Users/a b/проєкт/main.rs", "/home/u/src/lib.rs"] {
            let uri = path_to_uri(Path::new(path)).expect(path);
            assert!(uri.as_str().starts_with("file:///"), "{}", uri.as_str());
            assert_eq!(uri_to_path(&uri).unwrap(), PathBuf::from(path));
        }
        assert_eq!(path_to_uri(Path::new(r"C:\a\b.rs")).unwrap().as_str(), "file:///C:/a/b.rs");
        assert!(uri_to_path(&lsp_types::Uri::from_str("jdt://contents/x").unwrap()).is_none());
    }
}

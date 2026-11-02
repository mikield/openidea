//! Reformat Code, Rename and Find Usages against the server's language servers
//! (React: `src/editor/lsp/format.ts`, `rename.ts`, `features/code-insight/usages.ts`).
//!
//! Edits are never applied here: the server computes them (`lsp_format`, `lsp_rename`)
//! and applies them (`apply_workspace_edit`), and open documents take them back as
//! `DocPatched`, like any other backend change.

use client_rpc::{Connection, text};
use ide_core::{DocId, FileEdit, IdeResult, Location, Pos, Range, RequestId, TextEdit, WorkspaceEdit};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// `lsp_format`'s options for `text`: tabs when its lines are indented with tabs (React
/// reads the editor's indent unit, which it detects the same way).
fn format_options(text: &str) -> Value {
    let tabs = text.lines().any(|l| l.starts_with('\t'));
    json!({ "tabSize": 4, "insertSpaces": !tabs })
}

/// Formats document `doc` (at `path`, `text` synced): the number of edits applied.
pub async fn reformat(conn: Arc<Connection>, doc: DocId, path: PathBuf, text: String) -> IdeResult<usize> {
    let edits: Vec<TextEdit> =
        conn.invoke("lsp_format", json!({ "doc": doc, "range": null, "options": format_options(&text) })).await?;
    if !edits.is_empty() {
        let edit = WorkspaceEdit { files: vec![FileEdit::Edit { path: path.into(), edits: edits.clone() }] };
        apply(&conn, &edit).await?;
    }
    Ok(edits.len())
}

async fn apply(conn: &Connection, edit: &WorkspaceEdit) -> IdeResult<()> {
    conn.invoke::<Value>("apply_workspace_edit", json!({ "edit": edit })).await.map(drop)
}

/// The identifier around byte `offset` of `text` (the editor's word, for Rename's and
/// Find Usages' titles when the server has no range).
pub fn word_at(text: &str, offset: usize) -> std::ops::Range<usize> {
    let ident = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
    let offset = offset.min(text.len());
    let start = text[..offset].char_indices().rev().take_while(|(_, c)| ident(*c)).last().map_or(offset, |(i, _)| i);
    let end = text[offset..].char_indices().find(|(_, c)| !ident(*c)).map_or(text.len(), |(i, _)| offset + i);
    start..end
}

/// What `lsp_prepare_rename` allows renaming at `pos` of `text`, as a byte range; the word
/// at the caret when the server has no prepare step. `None`: nothing to rename there.
pub async fn prepare_rename(
    conn: Arc<Connection>,
    doc: DocId,
    text: String,
    pos: Pos,
) -> IdeResult<Option<std::ops::Range<usize>>> {
    let range: Option<Range> = conn.invoke("lsp_prepare_rename", json!({ "doc": doc, "pos": pos })).await?;
    let bytes = match range {
        Some(r) => text::pos_to_byte(&text, r.start)..text::pos_to_byte(&text, r.end),
        None => word_at(&text, text::pos_to_byte(&text, pos)),
    };
    Ok(Some(bytes).filter(|r| !r.is_empty()))
}

#[derive(Deserialize)]
struct Renamed {
    edit: WorkspaceEdit,
}

/// Renames the symbol at `pos` to `name`: (occurrences, files) changed.
pub async fn rename(conn: Arc<Connection>, doc: DocId, pos: Pos, name: String) -> IdeResult<(usize, usize)> {
    let renamed: Renamed = conn
        .invoke("lsp_rename", json!({ "request": RequestId::next(), "doc": doc, "pos": pos, "newName": name }))
        .await?;
    let occurrences = renamed
        .edit
        .files
        .iter()
        .map(|f| match f {
            FileEdit::Edit { edits, .. } => edits.len(),
            _ => 1,
        })
        .sum();
    let files = renamed.edit.files.len();
    if files > 0 {
        apply(&conn, &renamed.edit).await?;
    }
    Ok((occurrences, files))
}

/// One usage with its line, for the Find Usages tool window.
#[derive(Clone, Debug, PartialEq)]
pub struct Usage {
    pub range: Range,
    /// The usage's whole line; empty when the file could not be read.
    pub line: String,
}

/// Find Usages' answer: usages by file, in path and position order.
#[derive(Clone, Debug, Default)]
pub struct UsageResults {
    pub files: Vec<(PathBuf, Vec<Usage>)>,
    /// Why the list may be incomplete (`indexing`, `tooManyFiles`, `filesSkipped`).
    pub incomplete: Option<String>,
}

impl UsageResults {
    pub fn count(&self) -> usize {
        self.files.iter().map(|(_, u)| u.len()).sum()
    }
}

#[derive(Deserialize)]
struct References {
    locations: Vec<Location>,
    incomplete: Option<String>,
}

/// Every usage of the symbol at `pos`, project-wide and without its declaration (IDEA's
/// default), each with its line read through the server (`fs_read_text`).
pub async fn find_usages(conn: Arc<Connection>, doc: DocId, pos: Pos) -> IdeResult<UsageResults> {
    let refs: References = conn
        .invoke(
            "lsp_references",
            json!({
                "request": RequestId::next(),
                "doc": doc,
                "pos": pos,
                "includeDeclaration": false,
                "projectWide": true,
            }),
        )
        .await?;
    let mut by_file = BTreeMap::<PathBuf, Vec<Range>>::new();
    for l in refs.locations {
        by_file.entry(l.path.0).or_default().push(l.range);
    }
    let mut files = Vec::with_capacity(by_file.len());
    for (path, mut ranges) in by_file {
        ranges.sort_by_key(|r| (r.start.line, r.start.col));
        ranges.dedup();
        let text = read_text(&conn, &path).await;
        let lines: Vec<&str> = text.lines().collect();
        let usages = ranges
            .into_iter()
            .map(|range| Usage { range, line: lines.get(range.start.line as usize).unwrap_or(&"").to_string() })
            .collect();
        files.push((path, usages));
    }
    Ok(UsageResults { files, incomplete: refs.incomplete })
}

async fn read_text(conn: &Connection, path: &Path) -> String {
    conn.invoke::<String>("fs_read_text", json!({ "path": path })).await.unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_word_at_the_caret_spans_identifier_characters() {
        let text = "let foo_bar = baz(1);";
        assert_eq!(&text[word_at(text, 6)], "foo_bar");
        assert_eq!(&text[word_at(text, 4)], "foo_bar", "at its start");
        assert_eq!(&text[word_at(text, 11)], "foo_bar", "at its end");
        assert_eq!(&text[word_at(text, 15)], "baz");
        assert!(word_at(text, 12).is_empty(), "between words: nothing");
    }

    #[test]
    fn tab_indented_text_formats_with_tabs() {
        assert_eq!(format_options("fn x() {\n\t1\n}")["insertSpaces"], false);
        assert_eq!(format_options("fn x() {\n    1\n}")["insertSpaces"], true);
    }
}

//! Editor completion backed by the server's `lsp_completion`.

use crate::doc_sync::DocSync;
use client_rpc::proto::CompletionItem as WireItem;
use client_rpc::text;
use gpui::{App, Task, Window};
use gpui_component::Rope;
use gpui_component::input::CompletionProvider;
use lsp_types::{
    CompletionContext, CompletionItem, CompletionItemKind, CompletionList, CompletionResponse, CompletionTextEdit,
    InsertTextFormat, TextEdit,
};
use std::ops::Range;

pub struct ServerCompletions {
    pub sync: DocSync,
}

impl CompletionProvider for ServerCompletions {
    fn completions(
        &self,
        text: &Rope,
        offset: usize,
        trigger: CompletionContext,
        _window: &mut Window,
        cx: &mut App,
    ) -> Task<anyhow::Result<CompletionResponse>> {
        let source = text.to_string();
        let fut = self.sync.complete(source.clone(), offset, trigger.trigger_character);
        cx.background_executor().spawn(async move {
            let list = fut.await.map_err(|e| anyhow::anyhow!(e.message))?;
            // Without an explicit edit the editor inserts at the caret, keeping the
            // typed prefix: replace the identifier before the caret instead.
            let word = word_start(&source, offset)..offset;
            // Language servers leave filtering to the client and this editor does not
            // filter either (it only highlights the match), so rank here.
            let items = filter_and_rank(&source[word.clone()], list.items);
            Ok(CompletionResponse::List(CompletionList {
                is_incomplete: list.is_incomplete,
                items: items.into_iter().map(|item| to_lsp(item, &source, word.clone())).collect(),
            }))
        })
    }

    fn is_completion_trigger(&self, _offset: usize, new_text: &str, _cx: &mut App) -> bool {
        new_text == "." || new_text == ":" || new_text.chars().all(|c| c.is_alphanumeric() || c == '_')
    }
}

fn to_lsp(item: WireItem, source: &str, word: Range<usize>) -> CompletionItem {
    let insert_text = if item.is_snippet { strip_snippet(&item.insert_text) } else { item.insert_text };
    // The server's range is in UTF-16 columns of the text it was asked about
    // (`source`); the editor's `Position` counts chars.
    let bytes = item
        .replace_range
        .map(|r| text::pos_to_byte(source, r.start)..text::pos_to_byte(source, r.end))
        .unwrap_or(word);
    let range = lsp_types::Range::new(char_pos(source, bytes.start), char_pos(source, bytes.end));
    CompletionItem {
        label: item.label,
        kind: item.kind.as_deref().and_then(kind),
        detail: item.detail.or(item.label_detail),
        sort_text: item.sort_text,
        filter_text: item.filter_text,
        insert_text: Some(insert_text.clone()),
        insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
        text_edit: Some(CompletionTextEdit::Edit(TextEdit { range, new_text: insert_text })),
        ..Default::default()
    }
}

/// Items matching `query` (case-insensitive): prefix matches first, then
/// subsequence matches; each group in the server's `sort_text` order.
fn filter_and_rank(query: &str, items: Vec<WireItem>) -> Vec<WireItem> {
    let query = query.to_lowercase();
    let mut ranked: Vec<(u8, String, WireItem)> = items
        .into_iter()
        .filter_map(|item| {
            let key = item.filter_text.as_deref().unwrap_or(&item.label).to_lowercase();
            let class = if key.starts_with(&query) {
                0
            } else if is_subsequence(&query, &key) {
                1
            } else {
                return None;
            };
            let sort = item.sort_text.clone().unwrap_or_else(|| item.label.clone());
            Some((class, sort, item))
        })
        .collect();
    ranked.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
    ranked.into_iter().map(|(_, _, item)| item).collect()
}

fn is_subsequence(needle: &str, hay: &str) -> bool {
    let mut hay = hay.chars();
    needle.chars().all(|c| hay.any(|h| h == c))
}

/// Start of the identifier ending at byte `offset`.
fn word_start(text: &str, offset: usize) -> usize {
    text[..offset]
        .char_indices()
        .rev()
        .take_while(|(_, c)| c.is_alphanumeric() || *c == '_')
        .last()
        .map_or(offset, |(i, _)| i)
}

/// Editor position (line, char column) of byte `offset`.
fn char_pos(text: &str, offset: usize) -> lsp_types::Position {
    let line_start = text[..offset].rfind('\n').map_or(0, |i| i + 1);
    lsp_types::Position::new(
        text[..offset].matches('\n').count() as u32,
        text[line_start..offset].chars().count() as u32,
    )
}

/// Snippet text without tab stops (`${1:x}` → `x`, `$0` → ``): the editor
/// inserts plain text.
fn strip_snippet(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => out.extend(chars.next()),
            '$' if chars.peek() == Some(&'{') => {
                chars.next();
                while chars.peek().is_some_and(|c| c.is_ascii_digit()) {
                    chars.next();
                }
                if chars.peek() == Some(&':') {
                    chars.next();
                }
                // Placeholder text up to the matching `}`; it may hold nested tab stops.
                let mut inner = String::new();
                let mut depth = 1;
                while let Some(c) = chars.next() {
                    match c {
                        '\\' => {
                            inner.push(c);
                            inner.extend(chars.next());
                            continue;
                        }
                        '{' => depth += 1,
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    inner.push(c);
                }
                out.push_str(&strip_snippet(&inner));
            }
            '$' if chars.peek().is_some_and(|c| c.is_ascii_digit()) => {
                while chars.peek().is_some_and(|c| c.is_ascii_digit()) {
                    chars.next();
                }
            }
            c => out.push(c),
        }
    }
    out
}

fn kind(k: &str) -> Option<CompletionItemKind> {
    Some(match k {
        "text" => CompletionItemKind::TEXT,
        "method" => CompletionItemKind::METHOD,
        "function" => CompletionItemKind::FUNCTION,
        "constructor" => CompletionItemKind::CONSTRUCTOR,
        "field" => CompletionItemKind::FIELD,
        "variable" => CompletionItemKind::VARIABLE,
        "class" => CompletionItemKind::CLASS,
        "interface" => CompletionItemKind::INTERFACE,
        "module" => CompletionItemKind::MODULE,
        "property" => CompletionItemKind::PROPERTY,
        "value" => CompletionItemKind::VALUE,
        "enum" => CompletionItemKind::ENUM,
        "keyword" => CompletionItemKind::KEYWORD,
        "snippet" => CompletionItemKind::SNIPPET,
        "constant" => CompletionItemKind::CONSTANT,
        "struct" => CompletionItemKind::STRUCT,
        "enumMember" => CompletionItemKind::ENUM_MEMBER,
        "typeParameter" => CompletionItemKind::TYPE_PARAMETER,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    fn item(label: &str, sort: &str) -> super::WireItem {
        serde_json::from_value(serde_json::json!({
            "label": label, "sortText": sort, "insertText": label, "isSnippet": false
        }))
        .unwrap()
    }

    #[test]
    fn ranks_prefix_before_subsequence_and_drops_misses() {
        let items = vec![item("split_mut", "1"), item("is_empty", "2"), item("len", "3"), item("last", "0")];
        let labels: Vec<_> = super::filter_and_rank("le", items).into_iter().map(|i| i.label).collect();
        assert_eq!(labels, ["len"]);
        let items = vec![item("split_mut", "1"), item("len", "3"), item("last", "0")];
        let labels: Vec<_> = super::filter_and_rank("l", items).into_iter().map(|i| i.label).collect();
        assert_eq!(labels, ["last", "len", "split_mut"]);
        assert_eq!(super::filter_and_rank("", vec![item("a", "1")]).len(), 1);
    }

    #[test]
    fn replace_range_is_the_typed_word_in_chars() {
        let src = "fn f() {\n    привіт.len\n}";
        let caret = src.find("len").unwrap() + 3;
        let start = super::word_start(src, caret);
        assert_eq!(&src[start..caret], "len");
        assert_eq!(super::char_pos(src, start), lsp_types::Position::new(1, 11));
        assert_eq!(super::word_start("a.", 2), 2);
    }

    #[test]
    fn strip_snippet() {
        assert_eq!(super::strip_snippet("push(${1:value})$0"), "push(value)");
        assert_eq!(super::strip_snippet("vec![${1}]"), "vec![]");
        assert_eq!(super::strip_snippet("a\\$b"), "a$b");
        assert_eq!(super::strip_snippet("f(${1:x: ${2:i32}})"), "f(x: i32)");
    }
}

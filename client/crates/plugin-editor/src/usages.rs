//! The Find Usages tool window (`usages`, React `UsagesView`): the last search's
//! usages by file, each with its line and the usage marked. A click opens the
//! usage through [`client_core::Editors`].

use crate::code_insight::{Usage, UsageResults};
use client_core::icons::file_icon_name;
use client_core::theme::token;
use client_core::{Editors, Icon, Services};
use client_rpc::text;
use gpui::{
    App, ClickEvent, Context, Entity, InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_component::{h_flex, v_flex};
use ide_core::{IdeResult, Pos};
use std::path::{Path, PathBuf};

pub struct UsagesView {
    root: PathBuf,
    editors: Entity<Editors>,
    symbol: SharedString,
    state: State,
}

enum State {
    Idle,
    Searching,
    Done(UsageResults),
    Failed(SharedString),
}

fn usages(n: usize) -> String {
    match n {
        0 => "no usages".into(),
        1 => "1 usage".into(),
        n => format!("{n} usages"),
    }
}

/// `line` without its indent, split around the usage at UTF-16 columns `from..to`.
fn preview(line: &str, from: u32, to: u32) -> (&str, &str, &str) {
    let byte = |col| text::pos_to_byte(line, Pos::new(0, col)).min(line.len());
    let (from, to) = (byte(from), byte(to));
    let start = (line.len() - line.trim_start().len()).min(from);
    (&line[start..from], &line[from..to.max(from)], line[to.max(from)..].trim_end())
}

impl UsagesView {
    pub fn new(services: &Services, cx: &mut Context<Self>) -> Self {
        Self {
            root: services.backend.read(cx).project().root.0.clone(),
            editors: services.editors.clone(),
            symbol: SharedString::default(),
            state: State::Idle,
        }
    }

    /// Shows the search for `symbol` while `search` runs, then its usages.
    pub fn search(
        &mut self,
        symbol: SharedString,
        search: impl Future<Output = IdeResult<UsageResults>> + 'static,
        cx: &mut Context<Self>,
    ) {
        self.symbol = symbol;
        self.state = State::Searching;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = search.await;
            let _ = this.update(cx, |view, cx| {
                view.state = match result {
                    Ok(results) => State::Done(results),
                    Err(e) => State::Failed(e.message.into()),
                };
                cx.notify();
            });
        })
        .detach();
    }

    fn summary(&self) -> String {
        let of = if self.symbol.is_empty() { String::new() } else { format!(" of '{}'", self.symbol) };
        match &self.state {
            State::Idle => "Find Usages of the symbol at the caret".into(),
            State::Searching => format!("Searching for usages{of}…"),
            State::Failed(e) => format!("Usages{of}: {e}"),
            State::Done(r) => {
                let incomplete = match r.incomplete.as_deref() {
                    Some("indexing") => " (may be incomplete: still indexing)",
                    Some(_) => " (may be incomplete)",
                    None => "",
                };
                format!("Usages{of}: {}{incomplete}", usages(r.count()))
            }
        }
    }

    fn file_row(&self, path: &Path, count: usize, cx: &App) -> gpui::Div {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let dir = path.parent().map(|d| d.strip_prefix(&self.root).unwrap_or(d).display().to_string());
        h_flex()
            .h(px(24.))
            .px(px(8.))
            .gap(px(6.))
            .child(Icon::new(file_icon_name(&name)).size(px(16.)))
            .child(name)
            .child(div().text_color(token(cx, "fg-muted")).child(dir.unwrap_or_default()))
            .child(div().text_color(token(cx, "fg-subtle")).child(usages(count)))
    }

    fn usage_row(&self, ix: usize, path: &Path, usage: &Usage, cx: &App) -> gpui::Stateful<gpui::Div> {
        let (start, end) = (usage.range.start, usage.range.end);
        let to = if end.line == start.line { end.col } else { u32::MAX };
        let (before, matched, after) = preview(&usage.line, start.col, to);
        let (editors, open) = (self.editors.clone(), path.to_path_buf());
        let hover = token(cx, "bg-tree-hover");
        h_flex()
            .id(("usage", ix))
            .h(px(22.))
            .pl(px(30.))
            .pr(px(8.))
            .gap(px(10.))
            .hover(move |row| row.bg(hover))
            .child(
                div()
                    .w(px(36.))
                    .flex()
                    .justify_end()
                    .text_color(token(cx, "fg-muted"))
                    .child(format!("{}", start.line + 1)),
            )
            .child(
                h_flex()
                    .whitespace_nowrap()
                    .child(before.to_string())
                    .child(div().bg(token(cx, "bg-search-match")).child(matched.to_string()))
                    .child(after.to_string()),
            )
            .on_click(move |_: &ClickEvent, _, cx| {
                editors.update(cx, |e, cx| e.open_at(open.clone(), Some(start), cx));
            })
    }
}

impl Render for UsagesView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut rows = Vec::new();
        if let State::Done(results) = &self.state {
            let mut ix = 0;
            for (path, found) in &results.files {
                rows.push(self.file_row(path, found.len(), cx).into_any_element());
                for usage in found {
                    rows.push(self.usage_row(ix, path, usage, cx).into_any_element());
                    ix += 1;
                }
            }
        }
        v_flex()
            .size_full()
            .text_size(px(13.))
            .child(
                div()
                    .h(px(26.))
                    .flex_none()
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .text_color(token(cx, "fg-secondary"))
                    .child(self.summary()),
            )
            .child(div().id("usages").flex_1().min_h_0().overflow_y_scroll().children(rows))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previews_drop_the_indent_and_split_around_the_usage() {
        assert_eq!(preview("    let _x = greet();", 13, 18), ("let _x = ", "greet", "();"));
        assert_eq!(preview("é greet", 2, 7), ("é ", "greet", ""), "columns count UTF-16 units");
        assert_eq!(preview("short", 3, u32::MAX), ("sho", "rt", ""), "a usage running past the line");
    }
}

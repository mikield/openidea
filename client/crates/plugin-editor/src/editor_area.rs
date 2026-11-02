//! The editor area: open documents as tabs, each synced with the backend, with
//! completion and the problems the backend reports for it.

use crate::completion::ServerCompletions;
use crate::doc_sync::DocSync;
use client_core::icons::file_icon_name;
use client_core::keymap::shortcut_label;
use client_core::theme::token;
use client_core::{DiagnosticsChanged, DiagnosticsStore, EditorEvent, Editors, Icon, Services};
use client_rpc::proto::DocumentInfo;
use client_rpc::{Connection, text};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, AppContext, ClickEvent, Context, Entity, InteractiveElement, IntoElement, MouseButton, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, Subscription, Window, div, px, relative,
};
use gpui_component::highlighter::{Diagnostic as EditorDiagnostic, DiagnosticSeverity};
use gpui_component::input::{Editor, EditorState, InputEvent, Position};
use gpui_component::{h_flex, v_flex};
use ide_core::{IdeEvent, Pos, Severity};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use tokio::runtime::Handle;

pub struct EditorArea {
    conn: Arc<Connection>,
    rt: Handle,
    editors: Entity<Editors>,
    diagnostics: Entity<DiagnosticsStore>,
    docs: Vec<OpenDoc>,
    active: usize,
    /// What happened last (opening, saved, failed), shown beside the tabs.
    note: SharedString,
    _subscriptions: Vec<Subscription>,
}

struct OpenDoc {
    path: PathBuf,
    sync: DocSync,
    editor: Entity<EditorState>,
    dirty: bool,
    read_only: bool,
    _changes: Subscription,
}

/// The tab to show after closing tab `closed` of `len` while `active` was shown:
/// the same document if it survives, else its left neighbour (IDEA: "activate left").
fn after_close(active: usize, closed: usize, len: usize) -> Option<usize> {
    let remaining = len.checked_sub(1).filter(|n| *n > 0)?;
    Some(match active.cmp(&closed) {
        std::cmp::Ordering::Less => active,
        std::cmp::Ordering::Greater => active - 1,
        std::cmp::Ordering::Equal => closed.saturating_sub(1).min(remaining - 1),
    })
}

/// The tab `delta` away from `active`, wrapping around.
fn cycle(active: usize, len: usize, delta: isize) -> usize {
    (active as isize + delta).rem_euclid(len.max(1) as isize) as usize
}

impl EditorArea {
    pub fn new(services: &Services, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![
            cx.subscribe_in(&services.editors, window, |this, _, event: &EditorEvent, window, cx| {
                if let EditorEvent::OpenRequested(path) = event {
                    this.open_file(path.clone(), window, cx);
                }
            }),
            cx.subscribe(&services.diagnostics, |this, _, event: &DiagnosticsChanged, cx| {
                this.push_diagnostics(&event.path, cx)
            }),
            cx.subscribe(&services.backend, |this, _, event: &IdeEvent, cx| {
                if let IdeEvent::DocDirty { doc, dirty } = event
                    && let Some(open) = this.docs.iter_mut().find(|o| o.sync.doc == *doc)
                {
                    open.dirty = *dirty;
                    cx.notify();
                }
            }),
        ];
        let backend = services.backend.read(cx);
        Self {
            conn: backend.connection().clone(),
            rt: backend.runtime().clone(),
            editors: services.editors.clone(),
            diagnostics: services.diagnostics.clone(),
            docs: Vec::new(),
            active: 0,
            note: SharedString::default(),
            _subscriptions: subscriptions,
        }
    }

    pub fn open_file(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.docs.iter().position(|o| o.path == path) {
            return self.activate(ix, window, cx);
        }
        let fut = self.conn.doc_open(&path);
        self.note = format!("Opening {}…", path.display()).into();
        cx.spawn_in(window, async move |this, cx| {
            let result = fut.await;
            let _ = this.update_in(cx, |area, window, cx| match result {
                Ok(info) => area.show_doc(info, window, cx),
                Err(e) => {
                    area.note = format!("{}: {}", path.display(), e.message).into();
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn show_doc(&mut self, info: DocumentInfo, window: &mut Window, cx: &mut Context<Self>) {
        let path = info.path.0.clone();
        // Opened twice while the first request was in flight.
        if let Some(ix) = self.docs.iter().position(|o| o.path == path) {
            return self.activate(ix, window, cx);
        }
        let sync = DocSync::start(self.conn.clone(), &self.rt, &info);
        let language = info.language.clone();
        let read_only = info.read_only;
        let editor = cx.new(|cx| {
            let mut state = EditorState::new(window, cx).language(language);
            state.set_value(info.text.clone(), window, cx);
            state.set_readonly(read_only, cx);
            state.lsp_mut().completion_provider = Some(Rc::new(ServerCompletions { sync: sync.clone() }));
            state
        });
        let doc = info.id;
        let changes = cx.subscribe_in(&editor, window, move |area, editor, event: &InputEvent, _window, cx| {
            if let (InputEvent::Change, Some(open)) = (event, area.docs.iter().find(|o| o.sync.doc == doc)) {
                open.sync.edit(editor.read(cx).value().to_string());
            }
        });
        self.note = format!("{} · {}", info.language, if read_only { "read-only" } else { "editable" }).into();
        // New tabs open right of the current one, like IDEA.
        let at = if self.docs.is_empty() { 0 } else { self.active + 1 };
        self.docs
            .insert(at, OpenDoc { path: path.clone(), sync, editor, dirty: info.dirty, read_only, _changes: changes });
        self.push_diagnostics(&path, cx);
        // Recent Files (React marks a file when an editor opens it).
        drop(self.conn.invoke::<serde_json::Value>("mark_recent", serde_json::json!({ "path": path })));
        self.activate(at, window, cx);
    }

    fn activate(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(open) = self.docs.get(ix) else { return };
        self.active = ix;
        open.editor.update(cx, |state, cx| state.focus(window, cx));
        let path = open.path.clone();
        self.editors.update(cx, |editors, cx| editors.set_active(Some(path), cx));
        cx.notify();
    }

    /// Closes tab `ix`; dropping its sync closes the document on the backend.
    pub fn close(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.docs.len() {
            return;
        }
        let next = after_close(self.active, ix, self.docs.len());
        self.docs.remove(ix);
        match next {
            Some(next) => self.activate(next, window, cx),
            None => {
                self.active = 0;
                self.note = SharedString::default();
                self.editors.update(cx, |editors, cx| editors.set_active(None, cx));
                cx.notify();
            }
        }
    }

    /// `editor.closeTab`.
    pub fn close_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close(self.active, window, cx);
    }

    /// `editor.nextTab` / `editor.prevTab`.
    pub fn cycle(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        if !self.docs.is_empty() {
            self.activate(cycle(self.active, self.docs.len(), delta), window, cx);
        }
    }

    /// Saves the active document (the `file.save` action).
    pub fn save(&mut self, cx: &mut Context<Self>) {
        let Some(open) = self.docs.get(self.active) else { return };
        let fut = open.sync.save();
        cx.spawn(async move |this, cx| {
            let result = fut.await;
            let _ = this.update(cx, |area, cx| {
                area.note = match result {
                    Ok(()) => "Saved".into(),
                    Err(e) => format!("Save failed: {}", e.message).into(),
                };
                cx.notify();
            });
        })
        .detach();
    }

    /// Shows the problems of `path` in its editor, if it is open.
    fn push_diagnostics(&mut self, path: &Path, cx: &mut Context<Self>) {
        let Some(open) = self.docs.iter().find(|o| o.path == path) else { return };
        // In range order: the editor keeps them in a sum tree queried by visible range.
        let items = self.diagnostics.read(cx).problems().for_path(path);
        open.editor.update(cx, |state, cx| {
            let rope = state.text().clone();
            let text = rope.to_string();
            if let Some(set) = state.diagnostics_mut() {
                set.reset(&rope);
                for d in &items {
                    set.push(
                        EditorDiagnostic::new(
                            editor_pos(&text, d.range.start)..editor_pos(&text, d.range.end),
                            d.message.clone(),
                        )
                        .with_severity(severity(d.severity))
                        .with_source(d.source.clone().unwrap_or_default()),
                    );
                }
            }
            // Painting caches line styles until the next layout pass: request one.
            state.refresh(cx);
        });
    }

    fn tab(&self, ix: usize, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let open = &self.docs[ix];
        let active = ix == self.active;
        let name = open.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let (hover, fg_muted) = (token(cx, "bg-tab-hover"), token(cx, "fg-muted"));
        h_flex()
            .id(("editor-tab", ix))
            .h(px(28.))
            .pl(px(8.))
            .pr(px(4.))
            .gap(px(6.))
            .rounded(px(6.))
            .border_1()
            .border_color(gpui::transparent_black())
            .when(active, |t| t.bg(token(cx, "bg-tab-selected")).border_color(token(cx, "border-tab-selected")))
            .when(!active, |t| t.hover(move |t| t.bg(hover)))
            .child(Icon::new(file_icon_name(&name)).size(px(16.)))
            .child(name)
            .when(open.read_only, |t| t.child(div().text_color(fg_muted).child("read-only")))
            .child(
                div()
                    .id(("close-tab", ix))
                    .size(px(16.))
                    .rounded(px(4.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(12.))
                    .text_color(fg_muted)
                    .hover(move |b| b.bg(hover))
                    // A modified document shows a dot until hovered, like IDEA.
                    .child(if open.dirty { "●" } else { "✕" })
                    .on_click(cx.listener(move |area, _: &ClickEvent, window, cx| {
                        cx.stop_propagation();
                        area.close(ix, window, cx);
                    })),
            )
            .on_click(cx.listener(move |area, _: &ClickEvent, window, cx| area.activate(ix, window, cx)))
            .on_mouse_down(MouseButton::Middle, cx.listener(move |area, _, window, cx| area.close(ix, window, cx)))
    }
}

/// Protocol positions count UTF-16 units; the editor's `Position` counts chars.
fn editor_pos(text: &str, pos: Pos) -> Position {
    let byte = text::pos_to_byte(text, pos);
    let line_start = text[..byte].rfind('\n').map_or(0, |i| i + 1);
    Position::new(pos.line, text[line_start..byte].chars().count() as u32)
}

fn severity(s: Severity) -> DiagnosticSeverity {
    match s {
        Severity::Error => DiagnosticSeverity::Error,
        Severity::Warning => DiagnosticSeverity::Warning,
        Severity::Info => DiagnosticSeverity::Info,
        Severity::Hint => DiagnosticSeverity::Hint,
    }
}

/// What an empty editor area suggests (React's `EditorArea` empty state), with this keymap's keys.
fn empty_hints(cx: &App) -> gpui::Div {
    let hints = [
        ("Search Everywhere", "navigate.searchEverywhere"),
        ("Go to File", "navigate.gotoFile"),
        ("Recent Files", "navigate.recentFiles"),
        ("Navigation Bar", "navigate.navigationBar"),
    ];
    let (muted, link) = (token(cx, "fg-secondary"), token(cx, "link"));
    let rows = hints.into_iter().filter_map(|(title, action)| {
        let keys = shortcut_label(cx, action)?;
        Some(
            h_flex()
                .gap(px(16.))
                .child(div().w(px(160.)).flex().justify_end().text_color(muted).child(title))
                .child(div().w(px(160.)).text_color(link).child(keys)),
        )
    });
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap(px(10.))
        .text_size(px(13.))
        .children(rows)
        .child(div().pt(px(6.)).text_color(token(cx, "fg-subtle")).child("Open a file from the Project tool window"))
}

impl Render for EditorArea {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(open) = self.docs.get(self.active) else {
            return v_flex().size_full().child(empty_hints(cx));
        };
        let editor = open.editor.clone();
        let tabs: Vec<_> = (0..self.docs.len()).map(|ix| self.tab(ix, cx)).collect();
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .id("editor-tabs")
                    .h(px(38.))
                    .flex_none()
                    .px(px(6.))
                    .gap(px(2.))
                    .text_size(px(13.))
                    .overflow_x_scroll()
                    .children(tabs)
                    .child(
                        div().pl(px(8.)).text_size(px(12.)).text_color(token(cx, "fg-subtle")).child(self.note.clone()),
                    ),
            )
            .child(div().flex_1().min_h_0().child(Editor::new(&editor).bordered(false).h(relative(1.))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closing_a_tab_shows_its_left_neighbour_or_keeps_the_current_one() {
        assert_eq!(after_close(2, 2, 4), Some(1), "the shown tab closes: its left neighbour");
        assert_eq!(after_close(0, 0, 3), Some(0), "the first closes: the new first");
        assert_eq!(after_close(1, 3, 4), Some(1), "another tab closes: keep the shown one");
        assert_eq!(after_close(3, 1, 4), Some(2), "a tab to its left closes: it shifts left");
        assert_eq!(after_close(0, 0, 1), None, "the last tab closes");
    }

    #[test]
    fn tab_cycling_wraps() {
        assert_eq!(cycle(0, 3, -1), 2);
        assert_eq!(cycle(2, 3, 1), 0);
        assert_eq!(cycle(1, 3, 1), 2);
    }
}

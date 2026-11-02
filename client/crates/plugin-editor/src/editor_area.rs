//! The editor area: one open document (no tabs yet) synced with the backend,
//! with completion and the problems the backend reports for it.

use crate::completion::ServerCompletions;
use crate::doc_sync::DocSync;
use client_core::{DiagnosticsChanged, DiagnosticsStore, EditorEvent, Editors, Services};
use client_rpc::proto::DocumentInfo;
use client_rpc::{Connection, text};
use gpui::{
    AppContext, Context, Entity, IntoElement, ParentElement, Render, SharedString, Styled, Subscription, Window, div,
    px, relative,
};
use gpui_component::highlighter::{Diagnostic as EditorDiagnostic, DiagnosticSeverity};
use gpui_component::input::{Editor, EditorState, InputEvent, Position};
use gpui_component::{ActiveTheme, h_flex, v_flex};
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
    open: Option<OpenDoc>,
    /// What happened last (opening, saved, failed), shown beside the file name.
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
                    && let Some(open) = this.open.as_mut().filter(|o| o.sync.doc == *doc)
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
            open: None,
            note: SharedString::default(),
            _subscriptions: subscriptions,
        }
    }

    pub fn open_file(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if self.open.as_ref().is_some_and(|o| o.path == path) {
            return;
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
        let changes = cx.subscribe_in(&editor, window, |area, editor, event: &InputEvent, _window, cx| {
            if let (InputEvent::Change, Some(open)) = (event, &area.open) {
                open.sync.edit(editor.read(cx).value().to_string());
            }
        });
        editor.update(cx, |state, cx| state.focus(window, cx));
        let path = info.path.0.clone();
        self.note = format!("{} · {}", info.language, if read_only { "read-only" } else { "editable" }).into();
        self.open = Some(OpenDoc { path: path.clone(), sync, editor, dirty: info.dirty, read_only, _changes: changes });
        self.editors.update(cx, |editors, cx| editors.set_active(Some(path.clone()), cx));
        self.push_diagnostics(&path, cx);
        cx.notify();
    }

    /// Saves the open document (the `file.save` action).
    pub fn save(&mut self, cx: &mut Context<Self>) {
        let Some(open) = &self.open else { return };
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

    /// Shows the problems of `path` in its editor, if it is the open one.
    fn push_diagnostics(&mut self, path: &Path, cx: &mut Context<Self>) {
        let Some(open) = self.open.as_ref().filter(|o| o.path == path) else { return };
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

impl Render for EditorArea {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let title = match &self.open {
            Some(o) => {
                let name = o.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let flags = match (o.dirty, o.read_only) {
                    (_, true) => " (read-only)",
                    (true, _) => " ●",
                    _ => "",
                };
                format!("{name}{flags}")
            }
            None => "No file open".into(),
        };
        let body = match &self.open {
            Some(o) => div().size_full().child(Editor::new(&o.editor).bordered(false).h(relative(1.))),
            None => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme.muted_foreground)
                .child("Open a file from the Project tool window (Alt+1)"),
        };
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .h(px(30.))
                    .flex_none()
                    .px_3()
                    .gap_3()
                    .text_sm()
                    .bg(theme.tab_bar)
                    .border_b_1()
                    .border_color(theme.border)
                    .child(title)
                    .child(div().text_xs().text_color(theme.muted_foreground).child(self.note.clone())),
            )
            .child(div().flex_1().min_h_0().child(body))
    }
}

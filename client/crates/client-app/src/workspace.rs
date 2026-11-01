//! The project window: project tree, one editor, problems, status bar.

use crate::completion::ServerCompletions;
use crate::doc_sync::DocSync;
use client_rpc::proto::{DocumentInfo, FileEntry, ProjectInfo, ServerInfo};
use client_rpc::{Connection, text};
use gpui::{
    AppContext, ClickEvent, Context, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, Subscription, Window, actions, div, prelude::FluentBuilder, px,
    relative,
};
use gpui_component::highlighter::{Diagnostic as EditorDiagnostic, DiagnosticSeverity};
use gpui_component::input::{Editor, EditorState, InputEvent, Position};
use gpui_component::{ActiveTheme, h_flex, v_flex};
use ide_core::{Diagnostic, IdeEvent, Pos, Severity};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use tokio::runtime::Handle;
use tokio::sync::mpsc::UnboundedReceiver;

actions!(openidea, [Save]);

pub struct Workspace {
    conn: Arc<Connection>,
    rt: Handle,
    root: PathBuf,
    project: SharedString,
    server: SharedString,
    children: HashMap<PathBuf, Vec<FileEntry>>,
    expanded: HashSet<PathBuf>,
    rows: Vec<Row>,
    open: Option<OpenDoc>,
    /// `(path, producer)` → that producer's full set for the file.
    problems: BTreeMap<(PathBuf, String), Vec<Diagnostic>>,
    status: SharedString,
}

struct Row {
    depth: usize,
    entry: FileEntry,
}

struct OpenDoc {
    path: PathBuf,
    sync: DocSync,
    editor: Entity<EditorState>,
    dirty: bool,
    read_only: bool,
    _changes: Subscription,
}

impl Workspace {
    pub fn new(
        conn: Arc<Connection>,
        rt: Handle,
        server: ServerInfo,
        project: ProjectInfo,
        mut events: UnboundedReceiver<IdeEvent>,
        cx: &mut Context<Self>,
    ) -> Self {
        // Backend events: tokio's mpsc is executor-agnostic, so GPUI awaits it directly.
        cx.spawn(async move |this, cx| {
            while let Some(event) = events.recv().await {
                if this.update(cx, |ws, cx| ws.on_event(event, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        let root = project.root.0.clone();
        let mut ws = Self {
            conn,
            rt,
            root: root.clone(),
            project: project.name.into(),
            server: format!("openidea-server {} ({}/{})", server.version, server.os, server.arch).into(),
            children: HashMap::new(),
            expanded: HashSet::from([root.clone()]),
            rows: Vec::new(),
            open: None,
            problems: BTreeMap::new(),
            status: "Ready".into(),
        };
        ws.load_dir(root, cx);
        ws
    }

    // ── project tree ───────────────────────────────────────────────────────

    fn load_dir(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        let fut = self.conn.read_dir(&dir);
        cx.spawn(async move |this, cx| {
            let result = fut.await;
            let _ = this.update(cx, |ws, cx| {
                match result {
                    Ok(mut entries) => {
                        entries.retain(|e| e.name != ".git");
                        entries.sort_by(|a, b| b.is_dir().cmp(&a.is_dir()).then_with(|| a.name.cmp(&b.name)));
                        ws.children.insert(dir, entries);
                        ws.rebuild_rows();
                    }
                    Err(e) => ws.status = format!("{}: {}", dir.display(), e.message).into(),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn rebuild_rows(&mut self) {
        fn walk(ws: &Workspace, dir: &Path, depth: usize, out: &mut Vec<Row>) {
            for entry in ws.children.get(dir).into_iter().flatten() {
                out.push(Row { depth, entry: entry.clone() });
                if entry.is_dir() && ws.expanded.contains(&entry.path.0) {
                    walk(ws, &entry.path.0, depth + 1, out);
                }
            }
        }
        let mut rows = Vec::new();
        walk(self, &self.root, 0, &mut rows);
        self.rows = rows;
    }

    fn click_row(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.rows.get(ix) else { return };
        let path = row.entry.path.0.clone();
        if row.entry.is_dir() {
            if !self.expanded.remove(&path) {
                self.expanded.insert(path.clone());
                if !self.children.contains_key(&path) {
                    self.load_dir(path, cx);
                }
            }
            self.rebuild_rows();
            cx.notify();
        } else {
            self.open_file(path, window, cx);
        }
    }

    // ── documents ──────────────────────────────────────────────────────────

    pub fn open_file(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if self.open.as_ref().is_some_and(|o| o.path == path) {
            return;
        }
        let fut = self.conn.doc_open(&path);
        self.status = format!("Opening {}…", path.display()).into();
        cx.spawn_in(window, async move |this, cx| {
            let result = fut.await;
            let _ = this.update_in(cx, |ws, window, cx| match result {
                Ok(info) => ws.show_doc(info, window, cx),
                Err(e) => {
                    ws.status = format!("{}: {}", path.display(), e.message).into();
                    cx.notify();
                }
            });
        })
        .detach();
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
        let changes = cx.subscribe_in(&editor, window, |ws, editor, event: &InputEvent, _window, cx| {
            if let (InputEvent::Change, Some(open)) = (event, &ws.open) {
                open.sync.edit(editor.read(cx).value().to_string());
            }
        });
        editor.update(cx, |state, cx| state.focus(window, cx));
        let path = info.path.0.clone();
        self.status = format!("{} · {}", info.language, if read_only { "read-only" } else { "editable" }).into();
        self.open = Some(OpenDoc { path: path.clone(), sync, editor, dirty: info.dirty, read_only, _changes: changes });
        self.push_diagnostics(&path, cx);
        cx.notify();
    }

    fn save(&mut self, _: &Save, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(open) = &self.open else { return };
        let fut = open.sync.save();
        cx.spawn(async move |this, cx| {
            let result = fut.await;
            let _ = this.update(cx, |ws, cx| {
                ws.status = match result {
                    Ok(()) => "Saved".into(),
                    Err(e) => format!("Save failed: {}", e.message).into(),
                };
                cx.notify();
            });
        })
        .detach();
    }

    // ── backend events ─────────────────────────────────────────────────────

    fn on_event(&mut self, event: IdeEvent, cx: &mut Context<Self>) {
        match event {
            IdeEvent::Diagnostics { path, source, items } => {
                let path = path.0;
                if items.is_empty() {
                    self.problems.remove(&(path.clone(), source));
                } else {
                    self.problems.insert((path.clone(), source), items);
                }
                self.push_diagnostics(&path, cx);
            }
            IdeEvent::DocDirty { doc, dirty } => {
                if let Some(open) = self.open.as_mut().filter(|o| o.sync.doc == doc) {
                    open.dirty = dirty;
                }
            }
            IdeEvent::FsChanged { created, removed, .. } => {
                let dirs: HashSet<PathBuf> = created
                    .iter()
                    .chain(&removed)
                    .filter_map(|p| p.0.parent().map(Path::to_path_buf))
                    .filter(|d| self.children.contains_key(d))
                    .collect();
                for dir in dirs {
                    self.load_dir(dir, cx);
                }
            }
            IdeEvent::Task { done: true, .. } => self.status = "Ready".into(),
            IdeEvent::Task { title, message, fraction, .. } => {
                let pct = fraction.map(|f| format!(" {:.0}%", f * 100.0)).unwrap_or_default();
                self.status = format!("{title}{}{pct}", message.map(|m| format!(": {m}")).unwrap_or_default()).into();
            }
            _ => return,
        }
        cx.notify();
    }

    /// Shows the problems of `path` in its editor, if it is the open one.
    fn push_diagnostics(&mut self, path: &Path, cx: &mut Context<Self>) {
        let Some(open) = self.open.as_ref().filter(|o| o.path == path) else { return };
        let mut items: Vec<Diagnostic> = self
            .problems
            .range((path.to_path_buf(), String::new())..)
            .take_while(|((p, _), _)| p == path)
            .flat_map(|(_, v)| v.iter().cloned())
            .collect();
        // The editor keeps diagnostics in a sum tree queried by visible range: push in order.
        items.sort_by_key(|d| (d.range.start, d.range.end));
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
            if let Some(set) = state.diagnostics() {
                let ranges: Vec<_> = set.iter().map(|e| (e.range.clone(), e.severity)).collect();
                tracing::debug!("editor diagnostics: {ranges:?}");
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

impl Render for Workspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();

        let tree = div().id("project-tree").size_full().overflow_y_scroll().py_1().children(
            self.rows.iter().enumerate().map(|(ix, row)| {
                let dir = row.entry.is_dir();
                let marker = match (dir, self.expanded.contains(&row.entry.path.0)) {
                    (true, true) => "▾ ",
                    (true, false) => "▸ ",
                    _ => "  ",
                };
                let active = self.open.as_ref().is_some_and(|o| o.path == row.entry.path.0);
                div()
                    .id(ix)
                    .h(px(22.))
                    .pl(px(8. + row.depth as f32 * 14.))
                    .flex()
                    .items_center()
                    .text_sm()
                    .when(row.entry.ignored || row.entry.excluded, |d| d.text_color(theme.muted_foreground))
                    .when(active, |d| d.bg(theme.list_active))
                    .hover(|d| d.bg(theme.list_hover))
                    .child(format!("{marker}{}", row.entry.name))
                    .on_click(cx.listener(move |ws, _: &ClickEvent, window, cx| ws.click_row(ix, window, cx)))
            }),
        );

        let sidebar = v_flex()
            .w(px(260.))
            .h_full()
            .flex_none()
            .bg(theme.sidebar)
            .border_r_1()
            .border_color(theme.border)
            .child(
                div()
                    .px_2()
                    .py_1()
                    .text_xs()
                    .font_weight(FontWeight::BOLD)
                    .text_color(theme.muted_foreground)
                    .child(format!("PROJECT · {}", self.project)),
            )
            .child(div().flex_1().min_h_0().child(tree));

        let tab = match &self.open {
            Some(o) => {
                let name = o.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let flags = match (o.dirty, o.read_only) {
                    (_, true) => " (read-only)",
                    (true, _) => " ●",
                    _ => "",
                };
                format!("{name}{flags}")
            }
            None => "No file open — pick one in the project tree".into(),
        };

        let editor = match &self.open {
            Some(o) => div().size_full().child(Editor::new(&o.editor).bordered(false).h(relative(1.))),
            None => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme.muted_foreground)
                .child("Ctrl+S saves · completion via the server's language server"),
        };

        let problems: Vec<_> =
            self.problems.iter().flat_map(|((path, _), items)| items.iter().map(move |d| (path, d))).collect();
        let problems_panel = v_flex()
            .h(px(150.))
            .flex_none()
            .border_t_1()
            .border_color(theme.border)
            .child(
                div()
                    .px_2()
                    .py_1()
                    .text_xs()
                    .font_weight(FontWeight::BOLD)
                    .text_color(theme.muted_foreground)
                    .child(format!("PROBLEMS · {}", problems.len())),
            )
            .child(div().id("problems").flex_1().min_h_0().overflow_y_scroll().children(problems.into_iter().map(
                |(path, d)| {
                    let rel = path.strip_prefix(&self.root).unwrap_or(path);
                    let color = match d.severity {
                        Severity::Error => theme.danger,
                        Severity::Warning => theme.warning,
                        _ => theme.muted_foreground,
                    };
                    h_flex()
                        .px_2()
                        .gap_2()
                        .text_sm()
                        .child(div().text_color(color).child(format!("{:?}", d.severity)))
                        .child(format!("{}:{}:{}", rel.display(), d.range.start.line + 1, d.range.start.col + 1))
                        .child(
                            div()
                                .text_color(theme.muted_foreground)
                                .child(d.message.lines().next().unwrap_or("").to_owned()),
                        )
                },
            )));

        let status = h_flex()
            .h(px(24.))
            .flex_none()
            .px_2()
            .gap_3()
            .text_xs()
            .bg(theme.status_bar)
            .border_t_1()
            .border_color(theme.status_bar_border)
            .text_color(theme.muted_foreground)
            .child(self.server.clone())
            .child(div().child("·"))
            .child(self.status.clone());

        v_flex()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .key_context("Workspace")
            .on_action(cx.listener(Self::save))
            .child(
                h_flex().flex_1().min_h_0().child(sidebar).child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .child(
                            div()
                                .h(px(30.))
                                .flex_none()
                                .px_3()
                                .flex()
                                .items_center()
                                .text_sm()
                                .bg(theme.tab_bar)
                                .border_b_1()
                                .border_color(theme.border)
                                .child(tab),
                        )
                        .child(div().flex_1().min_h_0().child(editor))
                        .child(problems_panel),
                ),
            )
            .child(status)
    }
}

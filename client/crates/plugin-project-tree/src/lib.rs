//! The Project tool window (`toolwindow.project`, Alt+1): the project's files,
//! loaded a directory at a time, refreshed on `FsChanged`. A file opens through
//! [`client_core::Editors`]; the active one is highlighted.

use client_core::{Anchor, EditorEvent, Editors, Extensions, Plugin, Services, ToolWindow};
use client_rpc::Connection;
use client_rpc::proto::FileEntry;
use gpui::{
    App, AppContext, ClickEvent, Context, Entity, InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Window, div, prelude::FluentBuilder, px,
};
use gpui_component::ActiveTheme;
use ide_core::IdeEvent;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

pub struct ProjectTreePlugin;

impl Plugin for ProjectTreePlugin {
    fn id(&self) -> &'static str {
        "project-tree"
    }

    fn activate(&self, ext: &mut Extensions, _: &mut App) {
        ext.add_tool_window(ToolWindow {
            id: "project",
            title: "Project".into(),
            anchor: Anchor::Left,
            order: 0,
            build: Rc::new(|services, _, cx| cx.new(|cx| ProjectTree::new(services, cx)).into()),
        });
    }
}

/// One visible row: an entry and how deep it is nested.
#[derive(Debug, Clone)]
pub struct Row {
    pub depth: usize,
    pub entry: FileEntry,
}

/// Directories first, then by name; `.git` is not shown.
fn tidy(mut entries: Vec<FileEntry>) -> Vec<FileEntry> {
    entries.retain(|e| e.name != ".git");
    entries.sort_by(|a, b| b.is_dir().cmp(&a.is_dir()).then_with(|| a.name.cmp(&b.name)));
    entries
}

/// The rows a tree shows: `root`'s loaded children, descending into expanded directories.
fn visible_rows(root: &Path, children: &HashMap<PathBuf, Vec<FileEntry>>, expanded: &HashSet<PathBuf>) -> Vec<Row> {
    fn walk(
        dir: &Path,
        depth: usize,
        children: &HashMap<PathBuf, Vec<FileEntry>>,
        expanded: &HashSet<PathBuf>,
        out: &mut Vec<Row>,
    ) {
        for entry in children.get(dir).into_iter().flatten() {
            out.push(Row { depth, entry: entry.clone() });
            if entry.is_dir() && expanded.contains(&entry.path.0) {
                walk(&entry.path.0, depth + 1, children, expanded, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(root, 0, children, expanded, &mut out);
    out
}

pub struct ProjectTree {
    conn: Arc<Connection>,
    editors: Entity<Editors>,
    root: PathBuf,
    children: HashMap<PathBuf, Vec<FileEntry>>,
    expanded: HashSet<PathBuf>,
    rows: Vec<Row>,
    error: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

impl ProjectTree {
    pub fn new(services: &Services, cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![
            cx.subscribe(&services.backend, |this, _, event: &IdeEvent, cx| {
                if let IdeEvent::FsChanged { created, removed, .. } = event {
                    let dirs: HashSet<PathBuf> = created
                        .iter()
                        .chain(removed)
                        .filter_map(|p| p.0.parent().map(Path::to_path_buf))
                        .filter(|d| this.children.contains_key(d))
                        .collect();
                    for dir in dirs {
                        this.load_dir(dir, cx);
                    }
                }
            }),
            cx.subscribe(&services.editors, |_, _, event: &EditorEvent, cx| {
                if let EditorEvent::ActiveChanged(_) = event {
                    cx.notify();
                }
            }),
        ];
        let backend = services.backend.read(cx);
        let root = backend.project().root.0.clone();
        let mut tree = Self {
            conn: backend.connection().clone(),
            editors: services.editors.clone(),
            root: root.clone(),
            children: HashMap::new(),
            expanded: HashSet::from([root.clone()]),
            rows: Vec::new(),
            error: None,
            _subscriptions: subscriptions,
        };
        tree.load_dir(root, cx);
        tree
    }

    fn load_dir(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        let read = self.conn.read_dir(&dir);
        cx.spawn(async move |this, cx| {
            let result = read.await;
            let _ = this.update(cx, |tree, cx| {
                match result {
                    Ok(entries) => {
                        tree.children.insert(dir, tidy(entries));
                        tree.rows = visible_rows(&tree.root, &tree.children, &tree.expanded);
                    }
                    Err(e) => tree.error = Some(format!("{}: {}", dir.display(), e.message).into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn click_row(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(row) = self.rows.get(ix) else { return };
        let path = row.entry.path.0.clone();
        if row.entry.is_dir() {
            if !self.expanded.remove(&path) {
                self.expanded.insert(path.clone());
                if !self.children.contains_key(&path) {
                    self.load_dir(path, cx);
                }
            }
            self.rows = visible_rows(&self.root, &self.children, &self.expanded);
            cx.notify();
        } else {
            self.editors.update(cx, |editors, cx| editors.open(path, cx));
        }
    }
}

impl Render for ProjectTree {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let active = self.editors.read(cx).active().map(Path::to_path_buf);
        div()
            .id("project-tree")
            .size_full()
            .overflow_y_scroll()
            .py_1()
            .text_sm()
            .children(self.error.clone().map(|e| div().px_2().text_color(theme.danger).child(e)))
            .children(self.rows.iter().enumerate().map(|(ix, row)| {
                let dir = row.entry.is_dir();
                let marker = match (dir, self.expanded.contains(&row.entry.path.0)) {
                    (true, true) => "▾ ",
                    (true, false) => "▸ ",
                    _ => "  ",
                };
                div()
                    .id(ix)
                    .h(px(22.))
                    .pl(px(8. + row.depth as f32 * 14.))
                    .flex()
                    .items_center()
                    .when(row.entry.ignored || row.entry.excluded, |d| d.text_color(theme.muted_foreground))
                    .when(active.as_deref() == Some(row.entry.path.0.as_path()), |d| d.bg(theme.list_active))
                    .hover(|d| d.bg(theme.list_hover))
                    .child(format!("{marker}{}", row.entry.name))
                    .on_click(cx.listener(move |tree, _: &ClickEvent, _, cx| tree.click_row(ix, cx)))
            }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, dir: bool) -> FileEntry {
        let name = Path::new(path).file_name().unwrap().to_string_lossy().into_owned();
        serde_json::from_value(serde_json::json!({
            "name": name, "path": path, "kind": if dir { "dir" } else { "file" },
            "ignored": false, "excluded": false, "hidden": false,
        }))
        .unwrap()
    }

    #[test]
    fn rows_list_directories_first_and_descend_into_expanded_ones() {
        let root = PathBuf::from("/p");
        let children = HashMap::from([
            (root.clone(), tidy(vec![entry("/p/Cargo.toml", false), entry("/p/src", true), entry("/p/.git", true)])),
            (PathBuf::from("/p/src"), tidy(vec![entry("/p/src/main.rs", false)])),
        ]);
        let names = |rows: Vec<Row>| rows.into_iter().map(|r| (r.depth, r.entry.name)).collect::<Vec<_>>();

        let collapsed = HashSet::from([root.clone()]);
        assert_eq!(names(visible_rows(&root, &children, &collapsed)), [(0, "src".into()), (0, "Cargo.toml".into())]);

        let expanded = HashSet::from([root.clone(), PathBuf::from("/p/src")]);
        assert_eq!(
            names(visible_rows(&root, &children, &expanded)),
            [(0, "src".into()), (1, "main.rs".into()), (0, "Cargo.toml".into())]
        );
    }
}

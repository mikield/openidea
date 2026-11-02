//! The Project tool window (`toolwindow.project`, Alt+1): the project's files,
//! loaded a directory at a time, refreshed on `FsChanged`. A file opens through
//! [`client_core::Editors`]; the active one is highlighted.

use client_core::icons::file_icon_name;
use client_core::theme::token;
use client_core::{Anchor, EditorEvent, Editors, Extensions, Icon, Plugin, Services, ToolWindow};
use client_rpc::Connection;
use client_rpc::proto::FileEntry;
use gpui::{
    App, AppContext, ClickEvent, Context, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, Subscription, Window, div, prelude::FluentBuilder, px,
};
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
            icon: "toolwindow-project".into(),
            anchor: Anchor::Left,
            order: 0,
            own_header: false,
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

#[derive(Clone, Debug, serde::Deserialize)]
pub struct SourceFolder {
    /// Project-relative, `/`-separated; `""` is the root.
    pub path: String,
    /// `source` | `test` | `resources` | `testResources` | `excluded`.
    pub kind: String,
}

#[derive(serde::Deserialize)]
struct ProjectRoots {
    roots: Vec<SourceFolder>,
}

/// `path` relative to `root`, `/`-separated (`""` for the root).
fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root).map(|p| p.to_string_lossy().replace('\\', "/")).unwrap_or_default()
}

/// A folder's icon by role (React's `folderKind` without JVM packages / build output):
/// a marked root keeps its role; inside an excluded root (or excluded itself) it is excluded;
/// `.github` has its own icon.
fn folder_icon(roots: &[SourceFolder], rel: &str, name: &str, excluded: bool) -> &'static str {
    let role = |kind: &str| match kind {
        "source" => "folder-source",
        "test" => "folder-test",
        "resources" => "folder-resources",
        "testResources" => "folder-test-resources",
        _ => "folder-excluded",
    };
    if let Some(exact) = roots.iter().find(|r| r.path == rel) {
        return role(&exact.kind);
    }
    let inside_excluded =
        roots.iter().any(|r| r.kind == "excluded" && (r.path.is_empty() || rel.starts_with(&format!("{}/", r.path))));
    if excluded || inside_excluded {
        "folder-excluded"
    } else if name == ".github" {
        "folder-github"
    } else {
        "folder"
    }
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
    name: SharedString,
    /// Source / test / resources / excluded folders (`project_roots`), project-relative.
    roots: Vec<SourceFolder>,
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
                if let IdeEvent::ProjectRootsChanged { .. } = event {
                    this.load_roots(cx);
                }
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
            name: backend.project().name.clone().into(),
            roots: Vec::new(),
            children: HashMap::new(),
            expanded: HashSet::from([root.clone()]),
            rows: Vec::new(),
            error: None,
            _subscriptions: subscriptions,
        };
        tree.load_dir(root, cx);
        tree.load_roots(cx);
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

    fn load_roots(&mut self, cx: &mut Context<Self>) {
        let read = self.conn.invoke::<ProjectRoots>("project_roots", serde_json::json!({}));
        cx.spawn(async move |this, cx| {
            if let Ok(roots) = read.await {
                let _ = this.update(cx, |tree, cx| {
                    tree.roots = roots.roots;
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn toggle_root(&mut self, cx: &mut Context<Self>) {
        if !self.expanded.remove(&self.root) {
            self.expanded.insert(self.root.clone());
        }
        cx.notify();
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
        let active = self.editors.read(cx).active().map(Path::to_path_buf);
        let (hover, selected) = (token(cx, "bg-tree-hover"), token(cx, "bg-tree-selected"));
        let (muted, danger, chevron) = (token(cx, "fg-muted"), token(cx, "danger"), token(cx, "fg-icon"));
        let root_open = self.expanded.contains(&self.root);
        // The root node: module icon, project name, its path (React's project view).
        let root = row_frame("project-root", 0, hover)
            .child(Icon::new(if root_open { "chevron-down" } else { "chevron-right" }).size(px(16.)).color(chevron))
            .child(
                content()
                    .child(Icon::new("module").size(px(16.)))
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(self.name.clone()))
                    .child(
                        div()
                            .ml(px(4.))
                            .text_color(muted)
                            .truncate()
                            .child(self.root.display().to_string().replace('\\', "/")),
                    ),
            )
            .on_click(cx.listener(|tree, _: &ClickEvent, _, cx| tree.toggle_root(cx)));
        let rows = root_open.then(|| {
            self.rows
                .iter()
                .enumerate()
                .map(|(ix, row)| {
                    let dir = row.entry.is_dir();
                    let chevron = match (dir, self.expanded.contains(&row.entry.path.0)) {
                        (true, true) => Icon::new("chevron-down").size(px(16.)).color(chevron).into_any_element(),
                        (true, false) => Icon::new("chevron-right").size(px(16.)).color(chevron).into_any_element(),
                        _ => div().size(px(16.)).flex_none().into_any_element(),
                    };
                    let icon = if dir {
                        folder_icon(
                            &self.roots,
                            &relative(&self.root, &row.entry.path.0),
                            &row.entry.name,
                            row.entry.excluded,
                        )
                    } else {
                        file_icon_name(&row.entry.name)
                    };
                    row_frame(ix, row.depth + 1, hover)
                        .when(active.as_deref() == Some(row.entry.path.0.as_path()), |r| r.bg(selected))
                        .child(chevron)
                        .child(
                            content().child(Icon::new(icon).size(px(16.))).child(
                                div()
                                    .truncate()
                                    .when(row.entry.ignored || row.entry.excluded, |d| d.text_color(muted))
                                    .child(row.entry.name.clone()),
                            ),
                        )
                        .on_click(cx.listener(move |tree, _: &ClickEvent, _, cx| tree.click_row(ix, cx)))
                })
                .collect::<Vec<_>>()
        });
        div()
            .id("project-tree")
            .size_full()
            .overflow_y_scroll()
            .px(px(6.))
            .text_size(px(13.))
            .children(self.error.clone().map(|e| div().px_2().text_color(danger).child(e)))
            .child(root)
            .children(rows.into_iter().flatten())
    }
}

/// React's `.content` after the chevron: the icon and the label 5px apart.
fn content() -> gpui::Div {
    div().flex().items_center().gap(px(5.)).min_w_0()
}

/// A tree row: `--h-row`, 20px per level, rounded hover (`--radius-row`).
fn row_frame(id: impl Into<gpui::ElementId>, depth: usize, hover: gpui::Hsla) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .h(px(24.))
        .flex()
        .items_center()
        .gap(px(4.))
        .pl(px(4. + depth as f32 * 20.))
        .pr(px(8.))
        .rounded(px(5.))
        .hover(move |r| r.bg(hover))
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
    fn folders_show_their_role() {
        let root = |path: &str, kind: &str| SourceFolder { path: path.into(), kind: kind.into() };
        let roots = [root("src", "source"), root("tests", "test"), root("target", "excluded")];
        assert_eq!(folder_icon(&roots, "src", "src", false), "folder-source");
        assert_eq!(folder_icon(&roots, "tests", "tests", false), "folder-test");
        assert_eq!(folder_icon(&roots, "target/debug", "debug", false), "folder-excluded");
        assert_eq!(folder_icon(&roots, "src/ui", "ui", false), "folder");
        assert_eq!(folder_icon(&roots, ".github", ".github", false), "folder-github");
        assert_eq!(folder_icon(&roots, "node_modules", "node_modules", true), "folder-excluded");
        assert_eq!(relative(Path::new("/p"), Path::new("/p/src/ui")), "src/ui");
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

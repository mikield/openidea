//! Plugins and the extension points they contribute to.
//!
//! Every feature is a [`Plugin`]: on activation it registers contributions by
//! id into [`Extensions`] (a later registration with the same id replaces the
//! earlier one). The core only knows the extension points, never the features.
//! Behaviour follows the React workbench (`src/workbench/contributions.ts`).

use crate::actions::ActionDef;
use crate::keymap::KeymapPreset;
use crate::theme::ThemeDef;
use crate::{Backend, SettingsStore};
use gpui::{AnyView, App, Entity, Global, SharedString, Window};
use std::rc::Rc;

/// A feature: a UI crate that contributes to [`Extensions`].
pub trait Plugin: 'static {
    /// Stable id, e.g. `project-tree`.
    fn id(&self) -> &'static str;

    fn activate(&self, ext: &mut Extensions, cx: &mut App);
}

/// What a contributed view gets: the window's backend and settings.
#[derive(Clone)]
pub struct Services {
    pub backend: Entity<Backend>,
    pub settings: Entity<SettingsStore>,
}

/// Builds a contributed view for one window.
pub type ViewBuilder = Rc<dyn Fn(&Services, &mut Window, &mut App) -> AnyView>;

/// Where a tool window docks by default (the user may move it).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Anchor {
    Left,
    Right,
    Bottom,
}

/// A dockable tool window (Project, Problems, Terminal…).
#[derive(Clone)]
pub struct ToolWindow {
    /// Stable id; also the panel name its layout is saved under.
    pub id: &'static str,
    pub title: SharedString,
    pub anchor: Anchor,
    /// Order within its anchor; lower first.
    pub order: i32,
    pub build: ViewBuilder,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusAlign {
    Left,
    Right,
}

/// A status bar widget.
#[derive(Clone)]
pub struct StatusItem {
    pub id: &'static str,
    pub align: StatusAlign,
    /// Higher = further from the centre (closer to the window edge).
    pub priority: i32,
    pub build: ViewBuilder,
}

/// A page of the Settings dialog.
#[derive(Clone)]
pub struct SettingsPage {
    /// Dotted path defines nesting, e.g. `languages.servers`.
    pub id: &'static str,
    pub title: SharedString,
    /// Parent group title for top-level grouping, e.g. "Editor", "Tools".
    pub group: Option<SharedString>,
    pub order: i32,
    /// Extra words matched by the settings search.
    pub keywords: Vec<SharedString>,
    /// The page edits settings that a project can override (Global / This project switch).
    pub project_scope: bool,
    pub build: ViewBuilder,
}

/// Everything the activated plugins contributed. A GPUI global, installed by
/// [`crate::activate`].
#[derive(Default)]
pub struct Extensions {
    plugins: Vec<&'static str>,
    tool_windows: Vec<ToolWindow>,
    status_items: Vec<StatusItem>,
    settings_pages: Vec<SettingsPage>,
    editor_area: Option<ViewBuilder>,
    actions: Vec<ActionDef>,
    keymaps: Vec<KeymapPreset>,
    themes: Vec<ThemeDef>,
}

impl Global for Extensions {}

/// Replaces the entry with the same id, else appends.
fn upsert<T>(items: &mut Vec<T>, item: T, id: impl Fn(&T) -> &str) {
    match items.iter().position(|it| id(it) == id(&item)) {
        Some(ix) => items[ix] = item,
        None => items.push(item),
    }
}

impl Extensions {
    pub fn add_tool_window(&mut self, tool_window: ToolWindow) -> &mut Self {
        upsert(&mut self.tool_windows, tool_window, |t| t.id);
        self
    }

    pub fn add_status_item(&mut self, item: StatusItem) -> &mut Self {
        upsert(&mut self.status_items, item, |s| s.id);
        self
    }

    pub fn add_settings_page(&mut self, page: SettingsPage) -> &mut Self {
        upsert(&mut self.settings_pages, page, |p| p.id);
        self
    }

    /// The center of the workbench (React: `provideEditorArea`); the last one wins.
    pub fn set_editor_area(&mut self, build: ViewBuilder) -> &mut Self {
        self.editor_area = Some(build);
        self
    }

    pub fn add_action(&mut self, action: ActionDef) -> &mut Self {
        upsert(&mut self.actions, action, |a| &a.id);
        self
    }

    /// Ids of the activated plugins, in activation order.
    pub fn plugins(&self) -> &[&'static str] {
        &self.plugins
    }

    /// Tool windows by anchor, then order, then id.
    pub fn tool_windows(&self) -> Vec<&ToolWindow> {
        let mut out: Vec<_> = self.tool_windows.iter().collect();
        out.sort_by_key(|t| (t.anchor, t.order, t.id));
        out
    }

    pub fn tool_window(&self, id: &str) -> Option<&ToolWindow> {
        self.tool_windows.iter().find(|t| t.id == id)
    }

    /// Status items of one side, from the window edge towards the centre.
    pub fn status_items(&self, align: StatusAlign) -> Vec<&StatusItem> {
        let mut out: Vec<_> = self.status_items.iter().filter(|s| s.align == align).collect();
        out.sort_by_key(|s| (std::cmp::Reverse(s.priority), s.id));
        out
    }

    /// Settings pages by group, then order, then title.
    pub fn settings_pages(&self) -> Vec<&SettingsPage> {
        let mut out: Vec<_> = self.settings_pages.iter().collect();
        out.sort_by(|a, b| (&a.group, a.order, &a.title).cmp(&(&b.group, b.order, &b.title)));
        out
    }

    pub fn editor_area(&self) -> Option<&ViewBuilder> {
        self.editor_area.as_ref()
    }

    pub fn action(&self, id: &str) -> Option<&ActionDef> {
        self.actions.iter().find(|a| a.id == id)
    }

    /// Actions sorted by id.
    pub fn actions(&self) -> Vec<&ActionDef> {
        let mut out: Vec<_> = self.actions.iter().collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }

    /// A keymap preset; which one is used is the `keymap` setting.
    pub fn add_keymap(&mut self, preset: KeymapPreset) -> &mut Self {
        upsert(&mut self.keymaps, preset, |p| &p.id);
        self
    }

    pub fn keymaps(&self) -> &[KeymapPreset] {
        &self.keymaps
    }

    /// A theme; which one is used follows the appearance settings.
    pub fn add_theme(&mut self, theme: ThemeDef) -> &mut Self {
        upsert(&mut self.themes, theme, |t| &t.id);
        self
    }

    pub fn themes(&self) -> &[ThemeDef] {
        &self.themes
    }

    pub fn theme(&self, id: &str) -> Option<&ThemeDef> {
        self.themes.iter().find(|t| t.id == id)
    }
}

pub(crate) fn activate(plugins: &[Box<dyn Plugin>], cx: &mut App) {
    let mut ext = if cx.has_global::<Extensions>() { cx.remove_global::<Extensions>() } else { Extensions::default() };
    crate::workbench::contribute(&mut ext);
    for plugin in plugins {
        plugin.activate(&mut ext, cx);
        ext.plugins.push(plugin.id());
    }
    crate::workbench::contribute_tool_window_actions(&mut ext);
    cx.set_global(ext);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> ViewBuilder {
        Rc::new(|_, _, _| unreachable!("not built in these tests"))
    }

    fn tool(id: &'static str, anchor: Anchor, order: i32) -> ToolWindow {
        ToolWindow { id, title: id.into(), anchor, order, build: view() }
    }

    #[test]
    fn tool_windows_sort_by_anchor_then_order_and_replace_by_id() {
        let mut ext = Extensions::default();
        ext.add_tool_window(tool("terminal", Anchor::Bottom, 1))
            .add_tool_window(tool("structure", Anchor::Left, 2))
            .add_tool_window(tool("project", Anchor::Left, 1))
            .add_tool_window(tool("problems", Anchor::Bottom, 0))
            .add_tool_window(tool("structure", Anchor::Right, 0));
        let ids: Vec<_> = ext.tool_windows().iter().map(|t| (t.id, t.anchor)).collect();
        assert_eq!(
            ids,
            [
                ("project", Anchor::Left),
                ("structure", Anchor::Right),
                ("problems", Anchor::Bottom),
                ("terminal", Anchor::Bottom)
            ]
        );
    }

    #[test]
    fn status_items_run_from_the_edge_inwards() {
        let mut ext = Extensions::default();
        for (id, align, priority) in
            [("encoding", StatusAlign::Right, 10), ("position", StatusAlign::Right, 20), ("task", StatusAlign::Left, 0)]
        {
            ext.add_status_item(StatusItem { id, align, priority, build: view() });
        }
        let right: Vec<_> = ext.status_items(StatusAlign::Right).iter().map(|s| s.id).collect();
        assert_eq!(right, ["position", "encoding"]);
        assert_eq!(ext.status_items(StatusAlign::Left).len(), 1);
    }

    #[test]
    fn actions_replace_by_id() {
        let mut ext = Extensions::default();
        ext.add_action(ActionDef::new("file.save", "Save", |_, _, _| {}))
            .add_action(ActionDef::new("file.save", "Save All", |_, _, _| {}).category("File"));
        assert_eq!(ext.actions().len(), 1);
        let save = ext.action("file.save").unwrap();
        assert_eq!((save.title.as_ref(), save.category.as_deref()), ("Save All", Some("File")));
    }
}

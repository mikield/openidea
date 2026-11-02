//! The project window: the editor area in the center, tool windows docked
//! around it (left, right, bottom; tabs within each, draggable between them),
//! and a status bar. The layout is saved per project ([`LayoutStore`]) and
//! restored on the next open; tool windows added since then join their
//! default dock.

use crate::layout::LayoutStore;
use crate::plugin::{Anchor, Extensions, Services, StatusAlign, ToolWindow, ViewBuilder};
use crate::tool_window::ToolWindowPanel;
use crate::{ActionDef, RunAction, SettingsChanged, keymap, theme};
use gpui::{
    AnyView, App, AppContext, Context, Entity, FocusHandle, InteractiveElement, IntoElement, ParentElement, Pixels,
    Render, SharedString, Styled, Subscription, Task, Window, div, px,
};
use gpui_component::dock::{
    BasePanelView, DockArea, DockEvent, DockLayout, DockPlacement, DockSkin, PaneRef, PanelId, PanelState,
    panel_handle, register_panel,
};
use gpui_component::{ActiveTheme, h_flex, v_flex};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

/// The panel name the editor area is saved under.
pub const EDITOR_AREA: &str = "editor-area";

/// Bump when saved layouts can no longer be read; older ones are then dropped.
const LAYOUT_VERSION: usize = 1;

/// Layout edits come in bursts (a drag, a resize): save once they settle.
const SAVE_DELAY: Duration = Duration::from_millis(500);

const DOCKS: [(Anchor, DockPlacement, Pixels); 3] = [
    (Anchor::Left, DockPlacement::Left, px(260.)),
    (Anchor::Right, DockPlacement::Right, px(260.)),
    (Anchor::Bottom, DockPlacement::Bottom, px(200.)),
];

/// Tool window id (or [`EDITOR_AREA`]) → its panel in the dock.
type PanelIds = Rc<RefCell<HashMap<&'static str, PanelId>>>;

/// The workbench's own actions.
pub(crate) fn contribute(ext: &mut Extensions) {
    for (id, title, placement) in [
        ("workbench.toggleLeftDock", "Toggle Left Dock", DockPlacement::Left),
        ("workbench.toggleRightDock", "Toggle Right Dock", DockPlacement::Right),
        ("workbench.toggleBottomDock", "Toggle Bottom Dock", DockPlacement::Bottom),
    ] {
        ext.add_action(
            ActionDef::new(id, title, move |wb, window, cx| wb.toggle_dock(placement, window, cx)).category("View"),
        );
    }
    ext.add_action(
        ActionDef::new("workbench.restoreDefaultLayout", "Restore Default Layout", |wb, window, cx| {
            wb.default_layout(window, cx)
        })
        .category("Window"),
    );
    ext.add_action(
        ActionDef::new("toolwindow.hideAll", "Hide All Tool Windows", |wb, window, cx| wb.hide_all(window, cx))
            .category("Window"),
    );
}

/// `toolwindow.<id>` per tool window (React: `src/workbench/toolwindows/actions.ts`),
/// unless a plugin registered that id itself. Runs after every plugin activated.
pub(crate) fn contribute_tool_window_actions(ext: &mut Extensions) {
    let tool_windows: Vec<(&'static str, SharedString)> =
        ext.tool_windows().into_iter().map(|t| (t.id, t.title.clone())).collect();
    for (id, title) in tool_windows {
        let action = format!("toolwindow.{id}");
        if ext.action(&action).is_none() {
            ext.add_action(
                ActionDef::new(action, title, move |wb, window, cx| wb.toggle_tool_window(id, window, cx))
                    .category("Window"),
            );
        }
    }
}

pub struct Workbench {
    services: Services,
    dock: Entity<DockArea>,
    layout: LayoutStore,
    panels: PanelIds,
    status_left: Vec<AnyView>,
    status_right: Vec<AnyView>,
    focus: FocusHandle,
    save: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl Workbench {
    /// Builds the window from the activated plugins' contributions and restores
    /// the layout saved in `layout`, if any.
    pub fn new(services: Services, layout: LayoutStore, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let panels = PanelIds::default();
        register_builders(&services, &panels, cx);
        let (dock, _skin) = DockSkin::dock_area("workbench", Some(LAYOUT_VERSION), window, cx);
        let subscriptions = vec![
            cx.subscribe(&dock, |this, _, event: &DockEvent, cx| {
                if matches!(event, DockEvent::LayoutChanged) {
                    this.schedule_save(cx);
                }
            }),
            cx.on_app_quit(|this, cx| {
                this.save_layout(cx);
                async {}
            }),
            cx.subscribe_in(&services.settings, window, |this, _, _: &SettingsChanged, window, cx| {
                this.apply_appearance(window, cx)
            }),
            // "Sync with OS" follows the system's light / dark mode.
            cx.observe_window_appearance(window, |this, window, cx| this.apply_appearance(window, cx)),
        ];
        let (mut status_left, mut status_right) = (Vec::new(), Vec::new());
        if let Some(ext) = cx.try_global::<Extensions>() {
            let left: Vec<ViewBuilder> =
                ext.status_items(StatusAlign::Left).into_iter().map(|s| s.build.clone()).collect();
            let right: Vec<ViewBuilder> =
                ext.status_items(StatusAlign::Right).into_iter().map(|s| s.build.clone()).collect();
            status_left = left.iter().map(|build| build(&services, window, cx)).collect();
            // From the window edge inwards: the right edge comes last.
            status_right = right.iter().rev().map(|build| build(&services, window, cx)).collect();
        }
        let mut this = Self {
            services,
            dock,
            layout,
            panels,
            status_left,
            status_right,
            focus: cx.focus_handle(),
            save: None,
            _subscriptions: subscriptions,
        };
        this.apply_appearance(window, cx);
        this.restore_layout(window, cx);
        window.focus(&this.focus, cx);
        this
    }

    /// Binds the keymap and applies the theme the settings choose.
    fn apply_appearance(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let settings = self.services.settings.read(cx).settings().clone();
        let Some(ext) = cx.try_global::<Extensions>() else { return };
        let bindings = keymap::effective(ext.keymaps(), &settings);
        let actions: HashSet<String> = ext.actions().into_iter().map(|a| a.id.to_string()).collect();
        let theme_id =
            theme::resolve_theme_id(&settings.appearance, theme::system_light(window), |id| ext.theme(id).is_some());
        let chosen = ext.theme(theme_id).or_else(|| ext.themes().first()).cloned();
        keymap::apply(&bindings, |id| actions.contains(id), cx);
        if let Some(chosen) = chosen {
            theme::apply(&chosen, Some(&settings.appearance.accent), Some(window), cx);
        }
    }

    /// Closes every open dock (`toolwindow.hideAll`).
    pub fn hide_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dock.update(cx, |area, cx| {
            for (_, placement, _) in DOCKS {
                if area.is_dock_open(placement) {
                    area.toggle_dock(placement, window, cx);
                }
            }
        });
    }

    pub fn services(&self) -> &Services {
        &self.services
    }

    pub fn dock(&self) -> &Entity<DockArea> {
        &self.dock
    }

    /// Runs the action `id`; `false` when no plugin registered it.
    pub fn run_action(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(run) = cx.try_global::<Extensions>().and_then(|ext| ext.action(id)).map(|a| a.run.clone()) else {
            tracing::warn!("no action {id}");
            return false;
        };
        run(self, window, cx);
        true
    }

    pub fn toggle_dock(&mut self, placement: DockPlacement, window: &mut Window, cx: &mut Context<Self>) {
        self.dock.update(cx, |area, cx| area.toggle_dock(placement, window, cx));
    }

    /// Shows the tool window `id` (opening its dock, selecting its tab), or hides
    /// its dock when it is already the one shown.
    pub fn toggle_tool_window(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(panel) = self.panels.borrow().get(id).copied() else { return };
        let area = self.dock.read(cx);
        let Some(placement) = [DockPlacement::Center, DockPlacement::Left, DockPlacement::Right, DockPlacement::Bottom]
            .into_iter()
            .find(|p| area.layout(*p).is_some_and(|tree| tree.contains_panel(panel)))
        else {
            return;
        };
        let docked = placement != DockPlacement::Center;
        let open = area.is_dock_open(placement);
        let shown = docked && open && is_active(area, placement, panel);
        self.dock.update(cx, |area, cx| {
            if shown || (docked && !open) {
                area.toggle_dock(placement, window, cx);
            }
            if !shown {
                area.select_panel(panel, window, cx);
            }
        });
    }

    /// Whether the tool window `id` is on screen: its dock open and its tab selected.
    pub fn is_tool_window_visible(&self, id: &str, cx: &App) -> bool {
        let Some(panel) = self.panels.borrow().get(id).copied() else { return false };
        let area = self.dock.read(cx);
        DOCKS.iter().any(|(_, placement, _)| {
            area.layout(*placement).is_some_and(|tree| tree.contains_panel(panel))
                && area.is_dock_open(*placement)
                && is_active(area, *placement, panel)
        })
    }

    // ── layout ─────────────────────────────────────────────────────────────

    fn restore_layout(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = self.layout.load().filter(|s| s.version == Some(LAYOUT_VERSION)) else {
            return self.default_layout(window, cx);
        };
        self.panels.borrow_mut().clear();
        // Panels are rebuilt by name through the builders `register_builders` installed.
        match self.dock.update(cx, |area, cx| area.load(state, window, cx)) {
            Ok(()) => self.add_missing_panels(window, cx),
            Err(e) => {
                tracing::warn!("layout {}: {e}", self.layout.path().display());
                self.default_layout(window, cx);
            }
        }
    }

    /// Every tool window in its anchor's dock, in order; the editor area in the center.
    pub fn default_layout(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.panels.borrow_mut().clear();
        let (tool_windows, editor) = contributions(cx);
        let mut center = DockLayout::tabs();
        if let Some(build) = editor {
            let panel = build_editor_area(&build, &self.services, &self.panels, window, cx);
            center = center.panel_view(panel, cx);
        }
        let mut docks: BTreeMap<Anchor, DockLayout> = BTreeMap::new();
        for tool_window in &tool_windows {
            let panel = build_tool_window(tool_window, &self.services, &self.panels, window, cx);
            let tabs = docks.remove(&tool_window.anchor).unwrap_or_else(DockLayout::tabs);
            docks.insert(tool_window.anchor, tabs.panel_view(panel, cx));
        }
        self.dock.update(cx, |area, cx| {
            area.set_center(center, window, cx);
            for (anchor, placement, size) in DOCKS {
                match docks.remove(&anchor) {
                    Some(tabs) => {
                        area.set_dock(placement, tabs, window, cx);
                        area.set_dock_size(placement, size, window, cx);
                    }
                    None => area.remove_dock(placement, window, cx),
                }
            }
        });
    }

    /// Contributions the saved layout does not know yet go where a default layout puts them.
    fn add_missing_panels(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let saved = saved_panel_names(&self.dock.read(cx).dump(cx));
        let (tool_windows, editor) = contributions(cx);
        if let Some(build) = editor.filter(|_| !saved.contains(EDITOR_AREA)) {
            let panel = build_editor_area(&build, &self.services, &self.panels, window, cx);
            self.dock.update(cx, |area, cx| area.add_panel_view(panel, DockPlacement::Center, None, window, cx));
        }
        for tool_window in tool_windows.iter().filter(|t| !saved.contains(t.id)) {
            let panel = build_tool_window(tool_window, &self.services, &self.panels, window, cx);
            let (_, placement, size) =
                DOCKS.into_iter().find(|(a, ..)| *a == tool_window.anchor).expect("every anchor");
            self.dock.update(cx, |area, cx| area.add_panel_view(panel, placement, Some(size), window, cx));
        }
    }

    fn schedule_save(&mut self, cx: &mut Context<Self>) {
        // Replacing the task cancels the pending save: only the last edit of a burst writes.
        self.save = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SAVE_DELAY).await;
            let _ = this.update(cx, |this, cx| this.save_layout(cx));
        }));
    }

    pub(crate) fn save_layout(&mut self, cx: &mut Context<Self>) {
        self.save = None;
        let state = self.dock.read(cx).dump(cx);
        if let Err(e) = self.layout.save(&state) {
            tracing::warn!("saving layout {}: {e}", self.layout.path().display());
        }
    }

    fn on_run_action(&mut self, action: &RunAction, window: &mut Window, cx: &mut Context<Self>) {
        self.run_action(&action.id, window, cx);
    }
}

impl Render for Workbench {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let status = (!self.status_left.is_empty() || !self.status_right.is_empty()).then(|| {
            h_flex()
                .h(px(24.))
                .flex_none()
                .px_2()
                .gap_3()
                .text_xs()
                .bg(theme.status_bar)
                .border_t_1()
                .border_color(theme.status_bar_border)
                .text_color(theme.muted_foreground)
                .children(self.status_left.iter().cloned())
                .child(div().flex_1())
                .children(self.status_right.iter().cloned())
        });
        v_flex()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .key_context("Workbench")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::on_run_action))
            .child(div().flex_1().min_h_0().child(self.dock.clone()))
            .children(status)
    }
}

/// Whether `panel` is the selected tab of its group in `placement`.
fn is_active(area: &DockArea, placement: DockPlacement, panel: PanelId) -> bool {
    let Some(tree) = area.layout(placement) else { return false };
    tree.find_panel_node(panel).and_then(|node| tree.find_node(node)).is_some_and(
        |node| matches!(node.kind(), PaneRef::Tabs { panels, active_ix } if panels.get(active_ix) == Some(&panel)),
    )
}

fn contributions(cx: &App) -> (Vec<ToolWindow>, Option<ViewBuilder>) {
    match cx.try_global::<Extensions>() {
        Some(ext) => (ext.tool_windows().into_iter().cloned().collect(), ext.editor_area().cloned()),
        None => (Vec::new(), None),
    }
}

/// Panel names anywhere in a saved layout.
fn saved_panel_names(state: &gpui_component::dock::DockAreaState) -> HashSet<String> {
    fn walk(panel: &PanelState, out: &mut HashSet<String>) {
        out.insert(panel.panel_name.clone());
        for child in &panel.children {
            walk(child, out);
        }
    }
    let mut out = HashSet::new();
    walk(&state.center, &mut out);
    for dock in [&state.left_dock, &state.right_dock, &state.bottom_dock].into_iter().flatten() {
        walk(dock.panel(), &mut out);
    }
    out
}

/// Lets the dock rebuild contributed panels by name when it loads a saved layout.
fn register_builders(services: &Services, panels: &PanelIds, cx: &mut App) {
    let (tool_windows, editor) = contributions(cx);
    for tool_window in tool_windows {
        let (services, panels) = (services.clone(), panels.clone());
        register_panel(cx, tool_window.id, move |_, window, cx| {
            build_tool_window(&tool_window, &services, &panels, window, cx)
        });
    }
    if let Some(build) = editor {
        let (services, panels) = (services.clone(), panels.clone());
        register_panel(cx, EDITOR_AREA, move |_, window, cx| build_editor_area(&build, &services, &panels, window, cx));
    }
}

fn build_tool_window(
    tool_window: &ToolWindow,
    services: &Services,
    panels: &PanelIds,
    window: &mut Window,
    cx: &mut App,
) -> Arc<dyn BasePanelView> {
    let content = (tool_window.build)(services, window, cx);
    build_panel(tool_window.id, tool_window.title.clone(), true, content, panels, cx)
}

fn build_editor_area(
    build: &ViewBuilder,
    services: &Services,
    panels: &PanelIds,
    window: &mut Window,
    cx: &mut App,
) -> Arc<dyn BasePanelView> {
    let content = build(services, window, cx);
    build_panel(EDITOR_AREA, "Editor".into(), false, content, panels, cx)
}

fn build_panel(
    id: &'static str,
    title: SharedString,
    title_bar: bool,
    content: AnyView,
    panels: &PanelIds,
    cx: &mut App,
) -> Arc<dyn BasePanelView> {
    let panel = cx.new(|cx| ToolWindowPanel::new(id, title, title_bar, content, cx));
    let handle = panel_handle(panel);
    panels.borrow_mut().insert(id, handle.panel_id(cx));
    handle
}

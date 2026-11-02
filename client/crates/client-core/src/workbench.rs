//! The project window: the editor area in the center, tool windows docked
//! around it (left, right, bottom; tabs within each, draggable between them),
//! and a status bar. The layout is saved per project ([`LayoutStore`]) and
//! restored on the next open; tool windows added since then join their
//! default dock.

use crate::layout::LayoutStore;
use crate::plugin::{Anchor, Extensions, Services, StatusAlign, ToolWindow, ViewBuilder};
use crate::skin::Islands;
use crate::titlebar::WorkbenchTitleBar;
use crate::tool_window::ToolWindowPanel;
use crate::{ActionDef, HideToolWindow, Icon, MoveToolWindow, RunAction, SettingsChanged, keymap, theme};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyView, App, AppContext, ClickEvent, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, KeyDownEvent, ModifiersChangedEvent, MouseButton, ParentElement, Pixels, Render,
    SharedString, StatefulInteractiveElement, Styled, Subscription, Task, Window, div, px,
};
use gpui_component::dock::{
    BasePanelView, DockArea, DockEvent, DockLayout, DockPlacement, InsertTarget, PaneRef, PanelHandle, PanelId,
    PanelState, panel_handle, register_panel,
};
use gpui_component::{h_flex, v_flex};
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
    title_left: Vec<AnyView>,
    title_right: Vec<AnyView>,
    stripes: Vec<Stripe>,
    focus: FocusHandle,
    /// A popup over the window (Search Everywhere…) and its dismiss subscription.
    overlay: Option<(AnyView, Subscription)>,
    taps: keymap::DoubleTap,
    save: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl Workbench {
    /// Builds the window from the activated plugins' contributions and restores
    /// the layout saved in `layout`, if any.
    pub fn new(services: Services, layout: LayoutStore, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let panels = PanelIds::default();
        register_builders(&services, &panels, cx);
        let dock = cx.new(|cx| {
            DockArea::new("workbench", Some(LAYOUT_VERSION), window, cx).with_renderer(Rc::new(Islands::default()))
        });
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
        let (status, title, stripes) = match cx.try_global::<Extensions>() {
            Some(ext) => {
                let status = [StatusAlign::Left, StatusAlign::Right]
                    .map(|a| ext.status_items(a).into_iter().map(|s| s.build.clone()).collect::<Vec<_>>());
                let title = [StatusAlign::Left, StatusAlign::Right]
                    .map(|a| ext.title_items(a).into_iter().map(|t| t.build.clone()).collect::<Vec<_>>());
                let stripes: Vec<Stripe> = ext
                    .tool_windows()
                    .into_iter()
                    .map(|t| Stripe { id: t.id, title: t.title.clone(), icon: t.icon.clone(), anchor: t.anchor })
                    .collect();
                (status, title, stripes)
            }
            None => Default::default(),
        };
        // Each side from the window edge inwards: on the right the edge comes last.
        let build = |items: &[ViewBuilder], rev: bool, window: &mut Window, cx: &mut App| -> Vec<AnyView> {
            let mut views: Vec<AnyView> = items.iter().map(|b| b(&services, window, cx)).collect();
            if rev {
                views.reverse();
            }
            views
        };
        let (status_left, status_right) = (build(&status[0], false, window, cx), build(&status[1], true, window, cx));
        let (title_left, title_right) = (build(&title[0], false, window, cx), build(&title[1], true, window, cx));
        let mut this = Self {
            services,
            dock,
            layout,
            panels,
            status_left,
            status_right,
            title_left,
            title_right,
            stripes,
            focus: cx.focus_handle(),
            overlay: None,
            taps: keymap::DoubleTap::default(),
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
            let fonts = theme::Fonts::pick(
                &cx.text_system().all_font_names(),
                &settings.editor.font_family,
                settings.editor.font_size,
            );
            theme::apply(&chosen, Some(&settings.appearance.accent), &fonts, Some(window), cx);
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

    /// Shows `view` over the window and focuses it; it closes on [`DismissEvent`] or a click outside.
    pub fn show_overlay<V: Render + Focusable + EventEmitter<DismissEvent>>(
        &mut self,
        view: Entity<V>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dismissed =
            cx.subscribe_in(&view, window, |wb, _, _: &DismissEvent, window, cx| wb.close_overlay(window, cx));
        view.focus_handle(cx).focus(window, cx);
        self.overlay = Some((view.into(), dismissed));
        cx.notify();
    }

    pub fn close_overlay(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.overlay.take().is_some() {
            window.focus(&self.focus, cx);
            cx.notify();
        }
    }

    /// Double-tapped modifiers (`Shift Shift`): GPUI key bindings cannot express them.
    fn on_modifiers_changed(&mut self, event: &ModifiersChangedEvent, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(modifier) = self.taps.modifiers_changed(&event.modifiers, std::time::Instant::now())
            && let Some(action) = keymap::double_tap_action(cx, modifier)
        {
            self.run_action(&action, window, cx);
        }
    }

    fn on_run_action(&mut self, action: &RunAction, window: &mut Window, cx: &mut Context<Self>) {
        self.run_action(&action.id, window, cx);
    }
}

/// A tool window's button on a stripe.
#[derive(Clone)]
struct Stripe {
    id: &'static str,
    title: SharedString,
    icon: SharedString,
    anchor: Anchor,
}

/// `--stripe-button-w` is the hover area; the visible button is 32px, icons 20px.
const STRIPE_WIDTH: f32 = 40.;

/// A stripe button being dragged to another stripe.
#[derive(Clone)]
struct StripeDrag {
    id: &'static str,
    icon: SharedString,
}

impl Render for StripeDrag {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size(px(32.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.))
            .bg(theme::token(cx, "bg-stripe-selected"))
            .child(Icon::new(self.icon.clone()).size(px(20.)))
    }
}

impl Workbench {
    /// Where the tool window `id` docks now (the user may have moved it), else its anchor.
    fn current_anchor(&self, stripe: &Stripe, cx: &App) -> Anchor {
        let Some(panel) = self.panels.borrow().get(stripe.id).copied() else { return stripe.anchor };
        let area = self.dock.read(cx);
        DOCKS
            .iter()
            .find(|(_, placement, _)| area.layout(*placement).is_some_and(|tree| tree.contains_panel(panel)))
            .map_or(stripe.anchor, |(anchor, ..)| *anchor)
    }

    /// Moves the tool window `id` into the `to` dock (IDEA's "Move to"), showing it there.
    pub fn move_tool_window(&mut self, id: &str, to: Anchor, window: &mut Window, cx: &mut Context<Self>) {
        let Some(panel) = self.panels.borrow().get(id).copied() else { return };
        let (_, placement, size) = DOCKS.into_iter().find(|(a, ..)| *a == to).expect("every anchor");
        self.dock.update(cx, |area, cx| {
            let target = area.layout(placement).and_then(|tree| {
                let first = tree.panels().next()?;
                tree.find_panel_node(first)
            });
            match target {
                Some(node) => area.move_panel(panel, InsertTarget::Tabs { node, ix: None, activate: true }, window, cx),
                // An empty dock: take the panel out and start the dock with it.
                None => {
                    let Some(entity) = area
                        .panel(panel)
                        .and_then(|p| PanelHandle::of(p).map(|h| h.view()))
                        .and_then(|view| view.downcast::<ToolWindowPanel>().ok())
                    else {
                        return;
                    };
                    area.remove_panel(entity.clone(), window, cx);
                    area.add_panel_view(panel_handle(entity), placement, Some(size), window, cx);
                }
            }
            if !area.is_dock_open(placement) {
                area.toggle_dock(placement, window, cx);
            }
            area.select_panel(panel, window, cx);
        });
        cx.notify();
    }

    /// Hides the tool window `id` (IDEA's "Hide"): closes the dock showing it.
    pub fn hide_tool_window(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_tool_window_visible(id, cx) {
            self.toggle_tool_window(id, window, cx);
        }
    }

    fn stripe_button(&self, stripe: &Stripe, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let visible = self.is_tool_window_visible(stripe.id, cx);
        let (selected, hover) = (theme::token(cx, "bg-stripe-selected"), theme::token(cx, "bg-stripe-hover"));
        let id = stripe.id;
        let title = stripe.title.clone();
        let drag = StripeDrag { id, icon: stripe.icon.clone() };
        div()
            .id(SharedString::from(format!("stripe-{id}")))
            .size(px(32.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.))
            .when(visible, |b| b.bg(selected))
            .hover(move |b| b.bg(hover))
            .tooltip(move |window, cx| gpui_component::tooltip::Tooltip::new(title.clone()).build(window, cx))
            .child(Icon::new(stripe.icon.clone()).size(px(20.)))
            .on_click(cx.listener(move |wb, _: &ClickEvent, window, cx| {
                wb.toggle_tool_window(id, window, cx);
                cx.notify();
            }))
            .on_drag(drag, |drag, _, _, cx| cx.new(|_| drag.clone()))
    }

    /// A part of a stripe that takes dropped stripe buttons: they move to `anchor`.
    fn stripe_zone(&self, stripes: &[Stripe], anchor: Anchor, grow: bool, cx: &mut Context<Self>) -> gpui::Div {
        let accent = theme::token(cx, "accent-soft");
        v_flex()
            .w_full()
            .items_center()
            .gap(px(4.))
            .rounded(px(6.))
            .when(grow, |z| z.flex_1())
            .min_h(px(32.))
            .children(stripes.iter().map(|s| self.stripe_button(s, cx)).collect::<Vec<_>>())
            .drag_over::<StripeDrag>(move |z, _, _, _| z.bg(accent))
            .on_drop(
                cx.listener(move |wb, drag: &StripeDrag, window, cx| wb.move_tool_window(drag.id, anchor, window, cx)),
            )
    }

    /// A vertical stripe: `top` buttons, then `bottom` ones pushed to the end.
    fn stripe(
        &self,
        (top, top_anchor): (&[Stripe], Anchor),
        bottom: Option<(&[Stripe], Anchor)>,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        v_flex()
            .w(px(STRIPE_WIDTH))
            .flex_none()
            .h_full()
            .items_center()
            .p(px(4.))
            .gap(px(4.))
            .child(self.stripe_zone(top, top_anchor, true, cx))
            .children(bottom.map(|(stripes, anchor)| self.stripe_zone(stripes, anchor, false, cx)))
    }
}

impl Render for Workbench {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let anchors: Vec<(Stripe, Anchor)> =
            self.stripes.iter().map(|s| (s.clone(), self.current_anchor(s, cx))).collect();
        let at =
            |anchor: Anchor| anchors.iter().filter(|(_, a)| *a == anchor).map(|(s, _)| s.clone()).collect::<Vec<_>>();
        let (left, right, bottom) = (at(Anchor::Left), at(Anchor::Right), at(Anchor::Bottom));
        // IDEA's left stripe holds the left tool windows on top and the bottom ones below;
        // the right stripe is always there, to drop buttons on.
        let left_stripe = self.stripe((&left, Anchor::Left), Some((&bottom, Anchor::Bottom)), cx);
        let right_stripe = self.stripe((&right, Anchor::Right), None, cx);
        let status = h_flex()
            .h(px(30.))
            .flex_none()
            .px(px(12.))
            .gap(px(16.))
            .text_size(px(12.))
            .text_color(theme::token(cx, "fg-secondary"))
            .children(self.status_left.iter().cloned())
            .child(div().flex_1())
            .children(self.status_right.iter().cloned());
        v_flex()
            .relative()
            .size_full()
            .bg(theme::token(cx, "bg-frame"))
            .text_color(theme::token(cx, "fg"))
            .key_context("Workbench")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::on_run_action))
            .on_action(cx.listener(|wb, a: &MoveToolWindow, window, cx| wb.move_tool_window(&a.id, a.to, window, cx)))
            .on_action(cx.listener(|wb, a: &HideToolWindow, window, cx| wb.hide_tool_window(&a.id, window, cx)))
            .on_modifiers_changed(cx.listener(Self::on_modifiers_changed))
            .capture_key_down(cx.listener(|wb, _: &KeyDownEvent, _, _| wb.taps.key_pressed()))
            .child(WorkbenchTitleBar { left: self.title_left.clone(), right: self.title_right.clone() })
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(left_stripe)
                    .child(div().flex_1().min_w_0().h_full().child(self.dock.clone()))
                    .child(right_stripe),
            )
            .child(status)
            .children(self.overlay.as_ref().map(|(view, _)| {
                // Popups sit near the top centre, like IDEA's; a click beside one closes it.
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .flex()
                    .justify_center()
                    .pt(px(90.))
                    .on_mouse_down(MouseButton::Left, cx.listener(|wb, _, window, cx| wb.close_overlay(window, cx)))
                    .child(view.clone())
            }))
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
    build_panel(tool_window.id, tool_window.title.clone(), !tool_window.own_header, content, panels, cx)
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

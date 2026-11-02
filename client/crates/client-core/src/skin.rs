//! The Islands look of the dock, behaving like IDEA's tool windows (React:
//! `ToolWindows.module.css`, the "Islands" tokens of `tokens.css`):
//!
//! - every tab group is an island — rounded, on the frame colour, a gap apart;
//! - a tool window header: the bold title, then the options menu (Move to Left /
//!   Right / Bottom, Hide) and the hide button, right-aligned;
//! - the header drags the tool window onto another island (its header joins it,
//!   its content splits it, with a drop placeholder);
//! - the inner edge of each dock resizes it.

use crate::Icon;
use crate::actions::{HideToolWindow, MoveToolWindow};
use crate::plugin::Anchor;
use crate::theme::token;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext, Context, CursorStyle, Div, FontWeight, InteractiveElement, IntoElement, MouseButton,
    MouseMoveEvent, MouseUpEvent, ParentElement, Render, SharedString, Stateful, StatefulInteractiveElement, Styled,
    Window, canvas, div, px,
};
use gpui_base::ResizeHandleContext;
use gpui_component::Sizable as _;
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::dock::{
    DockAreaRenderer, DockContext, DockPlacement, DragPanel, DropIndicator, PanelHandle, TabGroupContext,
    TabGroupRenderer,
};
use gpui_component::menu::DropdownMenu as _;
use std::cell::Cell;
use std::rc::Rc;

/// `--island-gap`, `--island-radius`, `--h-header` at 100%.
const GAP: f32 = 6.;
const RADIUS: f32 = 10.;
const HEADER: f32 = 40.;

#[derive(Default)]
pub(crate) struct Islands {
    /// The dock whose edge is being dragged.
    resizing: Rc<Cell<Option<DockPlacement>>>,
}

impl DockAreaRenderer for Islands {
    fn frame(&self, _: &mut Window, cx: &mut App) -> Stateful<Div> {
        div().id("dock-area").bg(token(cx, "bg-frame"))
    }

    /// Splits inside a dock resize through their (invisible) gap: the frame shows through.
    fn render_split_handle(&self, _: &ResizeHandleContext, _: &mut Window, _: &mut App) -> Option<AnyElement> {
        Some(div().into_any_element())
    }

    fn render_dock(&self, dock: &DockContext, content: AnyElement, _: &mut Window, _: &mut App) -> AnyElement {
        let placement = dock.placement();
        // The handle sits in the gap on the dock's inner edge.
        let handle = div()
            .id(SharedString::from(format!("dock-resize-{placement:?}")))
            .absolute()
            .map(|h| match placement {
                DockPlacement::Left => {
                    h.top_0().right(px(-GAP / 2.)).h_full().w(px(GAP)).cursor(CursorStyle::ResizeLeftRight)
                }
                DockPlacement::Right => {
                    h.top_0().left(px(-GAP / 2.)).h_full().w(px(GAP)).cursor(CursorStyle::ResizeLeftRight)
                }
                _ => h.left_0().top(px(-GAP / 2.)).w_full().h(px(GAP)).cursor(CursorStyle::ResizeUpDown),
            })
            .on_mouse_down(MouseButton::Left, {
                let resizing = self.resizing.clone();
                move |_, _, cx| {
                    cx.stop_propagation();
                    resizing.set(Some(placement));
                }
            });
        // While dragging, the whole window follows the pointer, not just the handle.
        let tracker = canvas(|_, _, _| {}, {
            let (dock, resizing) = (dock.clone(), self.resizing.clone());
            move |_, _, window, _| {
                window.on_mouse_event({
                    let (dock, resizing) = (dock.clone(), resizing.clone());
                    move |event: &MouseMoveEvent, phase, window, cx| {
                        if phase.bubble() && resizing.get() == Some(placement) {
                            dock.resize_to(event.position, window, cx);
                        }
                    }
                });
                window.on_mouse_event(move |_: &MouseUpEvent, phase, window, cx| {
                    if phase.bubble() && resizing.get() == Some(placement) {
                        resizing.set(None);
                        dock.end_resize(window, cx);
                    }
                });
            }
        })
        .absolute()
        .size_0();
        div().relative().size_full().flex().child(content).child(handle).child(tracker).into_any_element()
    }

    fn tab_group_renderer(&self) -> Rc<dyn TabGroupRenderer> {
        Rc::new(Island)
    }
}

pub(crate) struct Island;

/// A panel's tab name, or its panel name when it is not one of ours.
fn title(group: &TabGroupContext, ix: usize, cx: &App) -> SharedString {
    let panel = &group.panels()[ix];
    PanelHandle::of(panel).and_then(|p| p.tab_name(cx)).unwrap_or_else(|| panel.panel_name(cx).into())
}

/// Whether the group's active panel wants the island header (the editor area and
/// tool windows that draw their own, like Terminal, do not).
fn wants_header(group: &TabGroupContext, cx: &App) -> bool {
    group.active_panel().and_then(PanelHandle::of).is_none_or(|p| p.title_bar(cx))
}

/// What a dragged tool window shows under the pointer.
struct DragPreview(SharedString);

impl Render for DragPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px(px(10.))
            .py(px(4.))
            .rounded(px(6.))
            .bg(token(cx, "bg-popup"))
            .border_1()
            .border_color(token(cx, "border-popup"))
            .text_size(px(13.))
            .text_color(token(cx, "fg"))
            .child(self.0.clone())
    }
}

/// The header's right side, as IDEA's tool window header: the options menu, Hide.
pub fn header_controls(id: SharedString, cx: &App) -> impl IntoElement + use<> {
    let muted = token(cx, "fg-icon");
    let menu_id = id.clone();
    div()
        .flex()
        .items_center()
        .gap(px(2.))
        .child(
            Button::new(SharedString::from(format!("tw-options-{id}")))
                .ghost()
                .xsmall()
                .child(Icon::new("more").size(px(16.)).color(muted))
                .dropdown_menu(move |menu, _, _| {
                    let id = menu_id.clone();
                    let to = |anchor: Anchor| Box::new(MoveToolWindow { id: id.clone(), to: anchor });
                    menu.menu("Move to Left", to(Anchor::Left))
                        .menu("Move to Right", to(Anchor::Right))
                        .menu("Move to Bottom", to(Anchor::Bottom))
                        .separator()
                        .menu("Hide", Box::new(HideToolWindow { id: id.clone() }))
                }),
        )
        .child(
            Button::new(SharedString::from(format!("tw-hide-{id}")))
                .ghost()
                .xsmall()
                .tooltip("Hide")
                .child(Icon::new("hide").size(px(16.)).color(muted))
                .on_click(move |_, window, cx| {
                    window.dispatch_action(Box::new(HideToolWindow { id: id.clone() }), cx);
                }),
        )
}

impl TabGroupRenderer for Island {
    fn frame(&self, _: &TabGroupContext, _: &mut Window, cx: &mut App) -> Stateful<Div> {
        div()
            .id("island")
            .m(px(GAP / 2.))
            .rounded(px(RADIUS))
            .overflow_hidden()
            .bg(token(cx, "bg-island"))
            .text_color(token(cx, "fg"))
            .text_size(px(13.))
    }

    fn render_tab_bar(&self, group: &TabGroupContext, _: &mut Window, cx: &mut App) -> AnyElement {
        if !wants_header(group, cx) {
            return div().into_any_element();
        }
        let active = group.active_ix();
        let id: SharedString = group.active_panel().map(|p| p.panel_name(cx)).unwrap_or_default().into();
        let (selected, hover, muted) =
            (token(cx, "bg-tab-selected"), token(cx, "bg-tab-hover"), token(cx, "fg-secondary"));
        let drop_border = token(cx, "accent");
        let single = group.panels().len() == 1;
        // One tool window: its title. Several sharing the island: their titles as tabs.
        let titles: Vec<AnyElement> = (0..group.panels().len())
            .map(|ix| {
                let select = group.clone();
                let name = title(group, ix, cx);
                div()
                    .id(("island-title", ix))
                    .px(px(if single { 0. } else { 8. }))
                    .py(px(4.))
                    .rounded(px(6.))
                    .when(ix == active, |t| t.font_weight(FontWeight::SEMIBOLD))
                    .when(!single && ix == active, |t| t.bg(selected))
                    .when(!single && ix != active, |t| t.text_color(muted).hover(move |t| t.bg(hover)))
                    .child(name.clone())
                    .on_click(move |_, window, cx| select.select_tab(ix, window, cx))
                    .when_some(group.drag_panel(ix, cx), move |t, drag| {
                        t.on_drag(drag, move |drag: &DragPanel, offset, _, cx| {
                            cx.stop_propagation();
                            drag.set_drag_offset(offset);
                            cx.new(|_| DragPreview(name.clone()))
                        })
                    })
                    .into_any_element()
            })
            .collect();
        let target = group.clone();
        div()
            .id("island-header")
            .h(px(HEADER))
            .flex_none()
            .flex()
            .items_center()
            .pl(px(12.))
            .pr(px(6.))
            .gap(px(4.))
            .children(titles)
            .child(div().flex_1().h_full())
            .child(header_controls(id, cx))
            // Dropping a tool window on a header puts it in this island.
            .when(group.is_droppable(), |h| {
                h.drag_over::<DragPanel>(move |h, _, _, _| h.border_b_2().border_color(drop_border)).on_drop(
                    move |drag: &DragPanel, window, cx| target.drop_panel(drag.clone(), None, true, window, cx),
                )
            })
            .into_any_element()
    }

    fn render_drop_indicator(&self, indicator: DropIndicator, _: &mut Window, cx: &mut App) -> Option<AnyElement> {
        let to = indicator.to();
        Some(
            div()
                .absolute()
                .left(to.origin().x)
                .top(to.origin().y)
                .w(to.size().width)
                .h(to.size().height)
                .rounded(px(RADIUS))
                .bg(token(cx, "accent-soft"))
                .border_2()
                .border_color(token(cx, "accent"))
                .into_any_element(),
        )
    }
}

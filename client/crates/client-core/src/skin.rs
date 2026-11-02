//! The Islands look of the dock (React: `ToolWindows.module.css`, the "Islands"
//! tokens of `tokens.css`): every tab group is an island — rounded, on the
//! frame colour, a gap apart — with a tool window header (bold title) instead
//! of a tab bar when it holds one panel.

use crate::theme::token;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, Div, FontWeight, InteractiveElement, IntoElement, ParentElement, Stateful,
    StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_base::ResizeHandleContext;
use gpui_component::dock::{DockAreaRenderer, PanelHandle, TabGroupContext, TabGroupRenderer};
use std::rc::Rc;

/// `--island-gap`, `--island-radius`, `--h-header` at 100%.
const GAP: f32 = 6.;
const RADIUS: f32 = 10.;
const HEADER: f32 = 40.;

pub(crate) struct Islands;

impl DockAreaRenderer for Islands {
    fn frame(&self, _: &mut Window, cx: &mut App) -> Stateful<Div> {
        div().id("dock-area").bg(token(cx, "bg-frame"))
    }

    /// The gap between islands is the frame showing through: no divider line.
    fn render_split_handle(&self, _: &ResizeHandleContext, _: &mut Window, _: &mut App) -> Option<AnyElement> {
        Some(div().into_any_element())
    }

    fn tab_group_renderer(&self) -> Rc<dyn TabGroupRenderer> {
        Rc::new(Islands)
    }
}

/// A panel's tab name, or its panel name when it is not one of ours.
fn title(group: &TabGroupContext, ix: usize, cx: &App) -> String {
    let panel = &group.panels()[ix];
    PanelHandle::of(panel)
        .and_then(|p| p.tab_name(cx))
        .map(|t| t.to_string())
        .unwrap_or_else(|| panel.panel_name(cx).to_owned())
}

/// Whether the group's active panel wants a header (the editor area draws its own tabs).
fn wants_header(group: &TabGroupContext, cx: &App) -> bool {
    group.active_panel().and_then(PanelHandle::of).is_none_or(|p| p.title_bar(cx))
}

impl TabGroupRenderer for Islands {
    fn frame(&self, _: &TabGroupContext, _: &mut Window, cx: &mut App) -> Stateful<Div> {
        div()
            .id("island")
            .m(px(GAP / 2.))
            .rounded(px(RADIUS))
            .overflow_hidden()
            .bg(token(cx, "bg-island"))
            .text_color(token(cx, "fg"))
    }

    fn render_tab_bar(&self, group: &TabGroupContext, _: &mut Window, cx: &mut App) -> AnyElement {
        if !wants_header(group, cx) {
            return div().into_any_element();
        }
        let header = div().h(px(HEADER)).flex_none().flex().items_center().px(px(12.)).gap(px(4.)).text_sm();
        if group.panels().len() == 1 {
            return header.font_weight(FontWeight::SEMIBOLD).child(title(group, 0, cx)).into_any_element();
        }
        // Several tool windows in one island: their titles as tabs.
        let (selected, hover) = (token(cx, "bg-tab-selected"), token(cx, "bg-tab-hover"));
        header
            .children((0..group.panels().len()).map(|ix| {
                let active = ix == group.active_ix();
                let select = group.clone();
                div()
                    .id(("island-tab", ix))
                    .px(px(8.))
                    .py(px(4.))
                    .rounded(px(6.))
                    .when(active, |tab| tab.bg(selected).font_weight(FontWeight::SEMIBOLD))
                    .hover(move |tab| tab.bg(hover))
                    .child(title(group, ix, cx))
                    .on_click(move |_, window, cx| select.select_tab(ix, window, cx))
            }))
            .into_any_element()
    }
}

//! The Terminal tool window (`toolwindow.terminal`, Alt+F12): shells on the
//! backend's PTYs, as tabs ("Local", "+" for another), like React's terminal.

mod emulator;
mod view;

use client_core::theme::token;
use client_core::{Anchor, Extensions, Plugin, Services, ToolWindow, tool_window_controls};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, AppContext, ClickEvent, Context, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement, Render,
    StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_component::{h_flex, v_flex};
use std::rc::Rc;
use view::TerminalView;

pub struct TerminalPlugin;

impl Plugin for TerminalPlugin {
    fn id(&self) -> &'static str {
        "terminal"
    }

    fn activate(&self, ext: &mut Extensions, _: &mut App) {
        ext.add_tool_window(ToolWindow {
            id: "terminal",
            title: "Terminal".into(),
            icon: "toolwindow-terminal".into(),
            anchor: Anchor::Bottom,
            order: 1,
            own_header: true,
            build: Rc::new(|services, _, cx| cx.new(|cx| TerminalPanel::new(services, cx)).into()),
        });
    }
}

pub struct TerminalPanel {
    services: Services,
    tabs: Vec<Entity<TerminalView>>,
    active: usize,
    next_number: usize,
}

/// React's terminal tab names: "Local", then "Local (2)", "Local (3)"…
fn tab_title(number: usize) -> String {
    if number <= 1 { "Local".into() } else { format!("Local ({number})") }
}

impl TerminalPanel {
    fn new(services: &Services, cx: &mut Context<Self>) -> Self {
        let mut panel = Self { services: services.clone(), tabs: Vec::new(), active: 0, next_number: 1 };
        panel.add_tab(cx);
        panel
    }

    fn add_tab(&mut self, cx: &mut Context<Self>) {
        let services = self.services.clone();
        let title = tab_title(self.next_number);
        self.next_number += 1;
        let view = cx.new(|cx| TerminalView::new(&services, title.into(), cx));
        cx.observe(&view, |_, _, cx| cx.notify()).detach();
        self.tabs.push(view);
        self.active = self.tabs.len() - 1;
        cx.notify();
    }

    fn close_tab(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix < self.tabs.len() {
            self.tabs.remove(ix);
            self.active = self.active.min(self.tabs.len().saturating_sub(1));
            cx.notify();
        }
    }
}

impl Render for TerminalPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (selected, hover, muted) = (token(cx, "bg-tab-selected"), token(cx, "bg-tab-hover"), token(cx, "fg-muted"));
        let tabs: Vec<_> = self
            .tabs
            .iter()
            .enumerate()
            .map(|(ix, view)| {
                let active = ix == self.active;
                h_flex()
                    .id(("terminal-tab", ix))
                    .h(px(24.))
                    .pl(px(8.))
                    .pr(px(4.))
                    .gap(px(4.))
                    .rounded(px(6.))
                    .when(active, |t| t.bg(selected))
                    .when(!active, |t| t.hover(move |t| t.bg(hover)))
                    .child(view.read(cx).title.clone())
                    .child(
                        div()
                            .id(("terminal-close", ix))
                            .px(px(3.))
                            .rounded(px(4.))
                            .text_size(px(12.))
                            .text_color(muted)
                            .hover(move |b| b.bg(hover))
                            .child("✕")
                            .on_click(cx.listener(move |panel, _: &ClickEvent, _, cx| {
                                cx.stop_propagation();
                                panel.close_tab(ix, cx);
                            })),
                    )
                    .on_click(cx.listener(move |panel, _: &ClickEvent, _, cx| {
                        panel.active = ix;
                        cx.notify();
                    }))
            })
            .collect();
        let new_tab = div()
            .id("terminal-new")
            .px(px(6.))
            .rounded(px(4.))
            .text_color(muted)
            .hover(move |b| b.bg(hover))
            .child("+")
            .on_click(cx.listener(|panel, _: &ClickEvent, _, cx| panel.add_tab(cx)));
        v_flex()
            .size_full()
            // React's header: the title, then the sessions as tabs, then the tool window controls.
            .child(
                h_flex()
                    .h(px(40.))
                    .flex_none()
                    .pl(px(12.))
                    .pr(px(6.))
                    .gap(px(4.))
                    .text_size(px(13.))
                    .child(div().pr(px(8.)).font_weight(FontWeight::SEMIBOLD).child("Terminal"))
                    .children(tabs)
                    .child(new_tab)
                    .child(div().flex_1())
                    .child(tool_window_controls("terminal".into(), cx)),
            )
            .child(div().flex_1().min_h_0().children(self.tabs.get(self.active).cloned()))
    }
}

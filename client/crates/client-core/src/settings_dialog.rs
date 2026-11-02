//! The Settings dialog (`file.settings`, React: `SettingsDialog.tsx`): the
//! contributed [`SettingsPage`]s by group on the left, the chosen page on the
//! right. Pages write through [`crate::SettingsStore`] as soon as a value
//! changes (React stages edits until OK / Apply; not yet here).

use crate::ActionDef;
use crate::plugin::{Extensions, Services, SettingsPage};
use crate::theme::token;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyView, App, AppContext, ClickEvent, Context, DismissEvent, EventEmitter, FocusHandle, Focusable, FontWeight,
    InteractiveElement, IntoElement, KeyDownEvent, MouseButton, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_component::{h_flex, v_flex};

pub(crate) fn contribute(ext: &mut Extensions) {
    ext.add_action(
        ActionDef::new("file.settings", "Settings…", |wb, window, cx| {
            let services = wb.services().clone();
            let dialog = cx.new(|cx| SettingsDialog::new(services, window, cx));
            wb.show_overlay(dialog, window, cx);
        })
        .category("File"),
    );
}

pub struct SettingsDialog {
    services: Services,
    pages: Vec<SettingsPage>,
    selected: usize,
    page: Option<AnyView>,
    focus: FocusHandle,
}

impl EventEmitter<DismissEvent> for SettingsDialog {}

impl Focusable for SettingsDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl SettingsDialog {
    fn new(services: Services, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let pages = cx
            .try_global::<Extensions>()
            .map(|e| e.settings_pages().into_iter().cloned().collect())
            .unwrap_or_default();
        let mut dialog = Self { services, pages, selected: 0, page: None, focus: cx.focus_handle() };
        dialog.select(0, window, cx);
        dialog
    }

    fn select(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(page) = self.pages.get(ix) else { return };
        self.selected = ix;
        self.page = Some((page.build)(&self.services, window, cx));
        cx.notify();
    }
}

impl Render for SettingsDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (muted, selected, hover) =
            (token(cx, "fg-muted"), token(cx, "bg-list-selected"), token(cx, "bg-tree-hover"));
        let mut nav: Vec<gpui::AnyElement> = Vec::new();
        let mut group: Option<SharedString> = None;
        for (ix, page) in self.pages.iter().enumerate() {
            if page.group != group {
                group = page.group.clone();
                if let Some(title) = &group {
                    nav.push(
                        div()
                            .px(px(10.))
                            .pt(px(10.))
                            .pb(px(2.))
                            .text_size(px(12.))
                            .text_color(muted)
                            .child(title.clone())
                            .into_any_element(),
                    );
                }
            }
            nav.push(
                div()
                    .id(("settings-page", ix))
                    .h(px(26.))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .rounded(px(4.))
                    .when(ix == self.selected, |r| r.bg(selected))
                    .when(ix != self.selected, |r| r.hover(move |r| r.bg(hover)))
                    .child(page.title.clone())
                    .on_click(cx.listener(move |dialog, _: &ClickEvent, window, cx| dialog.select(ix, window, cx)))
                    .into_any_element(),
            );
        }
        let title =
            self.pages.get(self.selected).map(|p| p.title.clone()).unwrap_or_else(|| "No settings pages".into());
        v_flex()
            .id("settings-dialog")
            .track_focus(&self.focus)
            .w(px(880.))
            .h(px(580.))
            .bg(token(cx, "bg-dialog"))
            .border_1()
            .border_color(token(cx, "border-popup"))
            .rounded(px(10.))
            .shadow_lg()
            .overflow_hidden()
            .text_size(px(13.))
            .text_color(token(cx, "fg"))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_key_down(cx.listener(|_, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    cx.emit(DismissEvent);
                }
            }))
            .child(
                h_flex()
                    .h(px(40.))
                    .flex_none()
                    .px(px(16.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Settings")
                    .child(div().flex_1())
                    .child(
                        div()
                            .id("settings-close")
                            .px(px(6.))
                            .rounded(px(4.))
                            .text_color(muted)
                            .hover(move |b| b.bg(hover))
                            .child("✕")
                            .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(DismissEvent))),
                    ),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_start()
                    .child(
                        div()
                            .id("settings-nav")
                            .w(px(220.))
                            .h_full()
                            .flex_none()
                            .px(px(6.))
                            .overflow_y_scroll()
                            .border_r_1()
                            .border_color(token(cx, "border"))
                            .children(nav),
                    )
                    .child(
                        v_flex()
                            .id("settings-page")
                            .flex_1()
                            .h_full()
                            .min_w_0()
                            .p(px(16.))
                            .gap(px(12.))
                            .overflow_y_scroll()
                            .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(title))
                            .children(self.page.clone()),
                    ),
            )
    }
}

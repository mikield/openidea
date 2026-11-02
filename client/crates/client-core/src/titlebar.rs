//! The frameless title bar (React: `TitleBar.tsx`): the project widget, the
//! plugins' title items (VCS branch, run configuration…), the window controls.

use crate::plugin::{Extensions, StatusAlign, TitleItem};
use crate::theme::token;
use crate::{Icon, RunAction};
use gpui::{
    AnyView, App, AppContext, Context, FontWeight, IntoElement, ParentElement, Render, RenderOnce, SharedString,
    Styled, Window, div, px,
};
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::{TitleBar, h_flex};
use std::rc::Rc;

/// `--project-color-N` slots, as React's `projectColor.ts` counts them.
const PROJECT_COLOR_COUNT: i64 = 9;

/// Stable 1-based colour slot for a project name (React's `projectColorIndex`: UTF-16 units, i32 wrapping).
pub fn project_color_index(name: &str) -> i64 {
    let h = name.encode_utf16().fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(i32::from(c)));
    i64::from(h).abs() % PROJECT_COLOR_COUNT + 1
}

/// IDEA's avatar initials (React's `projectInitials`): two camel-case capitals, else
/// the first letters of the first and last words, else one letter.
pub fn project_initials(name: &str) -> String {
    let text = name.trim();
    let camel = camel_initials(text);
    if camel.chars().count() == 2 {
        return camel;
    }
    for separators in [" ", ",", "-", "_", ".", "`'\""] {
        let words: Vec<&str> = text.split(|c| separators.contains(c)).filter(|w| !w.is_empty()).collect();
        if let [first, .., last] = words.as_slice() {
            let initial = |w: &str| w.chars().next().map(|c| c.to_uppercase().collect::<String>()).unwrap_or_default();
            return initial(first) + &initial(last);
        }
    }
    if camel.is_empty() { "?".into() } else { camel }
}

fn camel_initials(text: &str) -> String {
    let word: String = text.chars().skip_while(|c| !c.is_alphabetic()).take_while(|c| c.is_alphanumeric()).collect();
    let mut chars = word.chars();
    let Some(first) = chars.next() else { return String::new() };
    let capital = chars.find(|c| c.is_uppercase());
    first.to_uppercase().chain(capital).collect::<String>().to_uppercase()
}

#[derive(IntoElement)]
pub(crate) struct WorkbenchTitleBar {
    pub left: Vec<AnyView>,
    pub right: Vec<AnyView>,
}

impl RenderOnce for WorkbenchTitleBar {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        // `--h-titlebar`.
        TitleBar::new().h(px(40.)).bg(token(cx, "bg-frame")).border_color(token(cx, "bg-frame")).child(
            h_flex()
                .flex_1()
                .h_full()
                .gap(px(16.))
                .pl(px(10.))
                .pr(px(8.))
                .text_size(px(13.))
                .text_color(token(cx, "fg"))
                .children(self.left)
                .child(div().flex_1())
                .children(self.right),
        )
    }
}

/// The project widget: the avatar (colour slot, initials) and the name.
struct ProjectWidget(SharedString);

impl Render for ProjectWidget {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let avatar = token(cx, &format!("project-color-{}", project_color_index(&self.0)));
        // React's `.button`: 30px high, 8px padding and corners; the avatar's initials 11px semibold,
        // the name medium, then a 12px chevron.
        h_flex()
            .h(px(30.))
            .px(px(8.))
            .rounded(px(8.))
            .gap(px(6.))
            .child(
                div()
                    .size(px(20.))
                    .rounded(px(4.))
                    .bg(avatar)
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(token(cx, "fg-on-accent"))
                    .child(project_initials(&self.0)),
            )
            .child(div().font_weight(FontWeight::MEDIUM).child(self.0.clone()))
            .child(Icon::new("chevron-down").size(px(12.)).color(token(cx, "fg-icon")))
    }
}

/// A title bar icon button running the action `id` (React's search and settings buttons).
struct ActionButton {
    id: &'static str,
    icon: &'static str,
    tooltip: &'static str,
}

impl Render for ActionButton {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let id = self.id;
        Button::new(SharedString::from(format!("title-{id}")))
            .ghost()
            .size(px(30.))
            .rounded(px(8.))
            .tooltip(self.tooltip)
            .child(Icon::new(self.icon).size(px(18.)).color(token(cx, "fg-icon")))
            .on_click(move |_, window, cx| window.dispatch_action(Box::new(RunAction { id: id.into() }), cx))
    }
}

/// The title bar's own items, contributed like any plugin's: the project widget on the
/// left (a plugin's main menu goes before it), search and settings on the right.
pub(crate) fn contribute(ext: &mut Extensions) {
    ext.add_title_item(TitleItem {
        id: "project",
        align: StatusAlign::Left,
        priority: 100,
        build: Rc::new(|services, _, cx| {
            let name: SharedString = services.backend.read(cx).project().name.clone().into();
            cx.new(|_| ProjectWidget(name)).into()
        }),
    });
    for (id, action, icon, tooltip, priority) in [
        ("search", "navigate.searchEverywhere", "search", "Search Everywhere", 90),
        ("settings", "file.settings", "settings", "Settings", 100),
    ] {
        ext.add_title_item(TitleItem {
            id,
            align: StatusAlign::Right,
            priority,
            build: Rc::new(move |_, _, cx| cx.new(|_| ActionButton { id: action, icon, tooltip }).into()),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn avatar_matches_react() {
        assert_eq!(project_initials("py-demo"), "PD");
        assert_eq!(project_initials("MyProject"), "MP");
        assert_eq!(project_initials("idea"), "I");
        assert_eq!(project_initials("openidea"), "O");
        // `h = h * 31 + c` over UTF-16 units, |h| % 9 + 1 (JS reference values).
        for (name, slot) in [("a", 8), ("openidea", 4), ("client-rpc", 3), ("Проєкт", 5)] {
            assert_eq!(project_color_index(name), slot, "{name}");
        }
    }
}

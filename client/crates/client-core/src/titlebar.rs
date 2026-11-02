//! The frameless title bar (React: `TitleBar.tsx`): the project widget, the
//! plugins' title items (VCS branch, run configuration…), the window controls.

use crate::theme::token;
use gpui::{AnyView, App, Hsla, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window, div, px};
use gpui_component::{TitleBar, h_flex};

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
    pub project: SharedString,
    pub left: Vec<AnyView>,
    pub right: Vec<AnyView>,
}

impl RenderOnce for WorkbenchTitleBar {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let slot = project_color_index(&self.project);
        let avatar: Hsla = token(cx, &format!("project-color-{slot}"));
        // `--h-titlebar`.
        TitleBar::new().h(px(40.)).bg(token(cx, "bg-frame")).border_color(token(cx, "bg-frame")).child(
            h_flex()
                .flex_1()
                .h_full()
                .gap(px(12.))
                .pl(px(8.))
                .text_size(px(13.))
                .text_color(token(cx, "fg"))
                .child(
                    h_flex()
                        .gap(px(6.))
                        .child(
                            div()
                                .size(px(20.))
                                .rounded(px(4.))
                                .bg(avatar)
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_size(px(12.))
                                .text_color(token(cx, "fg-on-accent"))
                                .child(project_initials(&self.project)),
                        )
                        .child(self.project.clone()),
                )
                .children(self.left)
                .child(div().flex_1())
                .children(self.right),
        )
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

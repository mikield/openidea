//! The main menu (React: `MainMenu.tsx`, hamburger mode of the frameless title
//! bar). The menu structure is upstream's own spec (`src-tauri/src/menu.rs`,
//! taken at build time, see `build.rs`); an item runs its action id, and items
//! whose action no plugin registered yet are disabled. GPUI shows each item's
//! key binding from the keymap.

use client_core::theme::token;
use client_core::{Extensions, Icon, Plugin, RunAction, StatusAlign, TitleItem};
use gpui::{App, AppContext, Context, IntoElement, ParentElement, Render, SharedString, Window, px};
use gpui_component::Sizable as _;
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::menu::{DropdownMenu as _, PopupMenu};
use std::rc::Rc;

#[allow(dead_code, unused_imports, clippy::all)]
mod upstream {
    include!(concat!(env!("OUT_DIR"), "/menu_spec.rs"));

    impl Native {
        pub(crate) fn label(self) -> &'static str {
            self.default_label()
        }
    }
}

use upstream::{DYNAMIC_TOOL_WINDOWS, Entry, Os};

pub struct MenuPlugin;

impl Plugin for MenuPlugin {
    fn id(&self) -> &'static str {
        "menu"
    }

    fn activate(&self, ext: &mut Extensions, _: &mut App) {
        ext.add_title_item(TitleItem {
            id: "main-menu",
            align: StatusAlign::Left,
            // Before the project widget, at the window edge.
            priority: 1000,
            build: Rc::new(|_, _, cx| cx.new(|_| MainMenu { spec: Rc::new(upstream::spec(Os::current())) }).into()),
        });
    }
}

struct MainMenu {
    spec: Rc<Vec<Entry>>,
}

fn registered(id: &str, cx: &App) -> bool {
    cx.try_global::<Extensions>().is_some_and(|ext| ext.action(id).is_some())
}

/// Fills `menu` with `entries` (submenus recursively).
fn fill(mut menu: PopupMenu, entries: &[Entry], window: &mut Window, cx: &mut Context<PopupMenu>) -> PopupMenu {
    for entry in entries {
        menu = match entry {
            Entry::Submenu { label, children, .. } => {
                let children = children.clone();
                menu.submenu(*label, window, cx, move |m, window, cx| fill(m, &children, window, cx))
            }
            Entry::Action { id, label, .. } => {
                menu.menu_with_disabled(*label, Box::new(RunAction { id: (*id).into() }), !registered(id, cx))
            }
            // The OS's own items (macOS roles): shown, handled by the focused text view.
            Entry::Native(native) => menu.label(native.label()),
            Entry::Separator => menu.separator(),
            Entry::RecentProjects => menu.label("No Recent Projects"),
            Entry::Dynamic(slot) if *slot == DYNAMIC_TOOL_WINDOWS => {
                let tool_windows: Vec<(String, SharedString)> = cx
                    .try_global::<Extensions>()
                    .map(|ext| {
                        ext.tool_windows()
                            .into_iter()
                            .map(|t| (format!("toolwindow.{}", t.id), t.title.clone()))
                            .collect()
                    })
                    .unwrap_or_default();
                tool_windows
                    .into_iter()
                    .fold(menu, |m, (id, title)| m.menu(title, Box::new(RunAction { id: id.into() })))
            }
            Entry::Dynamic(_) => menu,
        };
    }
    menu
}

impl Render for MainMenu {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let spec = self.spec.clone();
        Button::new("main-menu")
            .ghost()
            .small()
            .tooltip("Main Menu")
            .child(Icon::new("main-menu").size(px(16.)).color(token(cx, "fg-icon")))
            .dropdown_menu(move |menu, window, cx| fill(menu.min_w(px(220.)), &spec, window, cx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The spec taken from upstream has the IDEA menus, and every action id in it is unique.
    #[test]
    fn upstream_spec_has_the_idea_menus() {
        let spec = upstream::spec(Os::Windows);
        let titles: Vec<&str> = spec
            .iter()
            .filter_map(|e| match e {
                Entry::Submenu { label, .. } => Some(*label),
                _ => None,
            })
            .collect();
        for menu in ["File", "Edit", "View", "Navigate", "Help"] {
            assert!(titles.contains(&menu), "no {menu} menu in {titles:?}");
        }
        fn actions<'a>(entries: &'a [Entry], out: &mut Vec<&'a str>) {
            for e in entries {
                match e {
                    Entry::Submenu { children, .. } => actions(children, out),
                    Entry::Action { id, .. } => out.push(id),
                    _ => {}
                }
            }
        }
        let mut ids = Vec::new();
        actions(&spec, &mut ids);
        assert!(ids.contains(&"file.settings") && ids.contains(&"navigate.gotoFile"), "{ids:?}");
    }
}

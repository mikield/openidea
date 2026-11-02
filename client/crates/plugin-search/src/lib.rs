//! Search Everywhere (Shift, Shift) and its Go to File / Class / Symbol / Action
//! entry points, as React's `src/features/search` registers them: one popup,
//! opened on the matching tab, over the backend's `find_files` /
//! `find_symbols` / `recent_files` and the registered actions.

mod model;
mod popup;

use client_core::{ActionDef, Extensions, Plugin};
use gpui::{App, AppContext};
use model::Tab;
use popup::SearchEverywhere;

pub struct SearchPlugin;

impl Plugin for SearchPlugin {
    fn id(&self) -> &'static str {
        "search"
    }

    fn activate(&self, ext: &mut Extensions, _: &mut App) {
        for (id, title, tab) in [
            ("navigate.searchEverywhere", "Search Everywhere", Tab::All),
            ("navigate.gotoClass", "Go to Class", Tab::Classes),
            ("navigate.gotoFile", "Go to File", Tab::Files),
            ("navigate.gotoSymbol", "Go to Symbol", Tab::Symbols),
            ("navigate.findAction", "Find Action", Tab::Actions),
            // React's Recent Files is its own switcher; until then, the Files tab's recent list.
            ("navigate.recentFiles", "Recent Files", Tab::Files),
        ] {
            ext.add_action(
                ActionDef::new(id, title, move |wb, window, cx| {
                    let services = wb.services().clone();
                    let workbench = cx.entity().downgrade();
                    let popup = cx.new(|cx| SearchEverywhere::new(&services, workbench, tab, window, cx));
                    wb.show_overlay(popup, window, cx);
                })
                .category("Navigate"),
            );
        }
    }
}

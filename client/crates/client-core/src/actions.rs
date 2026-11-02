//! Actions ("everything is an action"): a feature registers its behaviour once
//! under a stable dotted id (`file.save`, `workbench.toggleLeftDock`), and key
//! bindings, menus and the command palette all resolve to that id.
//!
//! GPUI dispatches typed actions; every binding here dispatches the one typed
//! [`RunAction`] carrying the id, and the [`Workbench`](crate::Workbench) looks
//! the id up. So keymaps stay data (id → shortcuts), as in the React keymaps.

use crate::Workbench;
use gpui::{Context, SharedString, Window};
use std::rc::Rc;

/// Runs the action with `id`. Bound to keystrokes by [`Extensions::bind`](crate::Extensions::bind).
#[derive(Clone, PartialEq, Debug, gpui::Action)]
#[action(namespace = openidea, no_json)]
pub struct RunAction {
    pub id: SharedString,
}

/// What an action does. It runs inside the workbench's update, so it can reach
/// the dock, the tool windows and the services directly.
pub type ActionRun = Rc<dyn Fn(&mut Workbench, &mut Window, &mut Context<Workbench>)>;

#[derive(Clone)]
pub struct ActionDef {
    /// Dotted, stable id, e.g. `file.save`.
    pub id: &'static str,
    pub title: SharedString,
    /// Group in the command palette and the keymap editor, e.g. `File`.
    pub category: Option<SharedString>,
    pub run: ActionRun,
}

impl ActionDef {
    pub fn new(
        id: &'static str,
        title: impl Into<SharedString>,
        run: impl Fn(&mut Workbench, &mut Window, &mut Context<Workbench>) + 'static,
    ) -> Self {
        Self { id, title: title.into(), category: None, run: Rc::new(run) }
    }

    pub fn category(mut self, category: impl Into<SharedString>) -> Self {
        self.category = Some(category.into());
        self
    }
}

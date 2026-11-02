//! The workbench core: a small set of extension points that every feature
//! plugs into, plus the services they share.
//!
//! - [`Plugin`] / [`Extensions`]: tool windows, status bar items, settings
//!   pages, the editor area and actions, contributed by id (React:
//!   `src/workbench/contributions.ts`).
//! - [`ActionDef`] / [`RunAction`]: actions with stable dotted ids; key bindings
//!   name an id, never a Rust type (React: `src/platform/actions.ts`).
//! - [`Workbench`]: the window: a dock with tool windows and tabs around the
//!   editor area, and a status bar. The layout persists per project.
//! - [`Backend`] / [`SettingsStore`]: the window's server connection and its
//!   settings (`settings_get` / `settings_set`).

mod actions;
mod backend;
pub mod layout;
mod plugin;
mod settings;
mod tool_window;
mod workbench;
#[cfg(test)]
mod workbench_tests;

pub use actions::{ActionDef, ActionRun, RunAction};
pub use backend::Backend;
pub use plugin::{
    Anchor, Extensions, Plugin, Services, SettingsPage, StatusAlign, StatusItem, ToolWindow, ViewBuilder,
};
pub use settings::{SettingsChanged, SettingsScope, SettingsStore};
pub use workbench::{EDITOR_AREA, Workbench};

/// Activates `plugins` in order and installs what they contribute.
/// Call once, after `gpui_component::init`, before opening a [`Workbench`].
pub fn activate(plugins: &[Box<dyn Plugin>], cx: &mut gpui::App) {
    plugin::activate(plugins, cx);
}

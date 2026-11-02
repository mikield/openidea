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
//! - [`KeymapPreset`] / [`ThemeDef`]: keymaps and themes are contributed as data;
//!   the settings pick one of each.
//! - [`Services`]: what plugins share — the window's [`Backend`], its [`SettingsStore`]
//!   (`settings_get` / `settings_set`), [`Editors`] (open a file, which one is active) and
//!   [`DiagnosticsStore`] (problems per file).

mod actions;
mod assets;
mod backend;
pub mod diagnostics;
pub mod editors;
pub mod icons;
pub mod keymap;
pub mod layout;
mod plugin;
mod settings;
mod settings_dialog;
pub(crate) mod skin;
mod status;
pub mod theme;
mod titlebar;
mod tool_window;
mod workbench;
#[cfg(test)]
mod workbench_tests;

pub use actions::{ActionDef, ActionRun, HideToolWindow, MoveToolWindow, RunAction};
pub use assets::{Assets, register_fonts};
pub use backend::Backend;
pub use diagnostics::{DiagnosticsChanged, DiagnosticsStore};
pub use editors::{EditorEvent, Editors};
pub use icons::Icon;
pub use keymap::KeymapPreset;
pub use plugin::{
    Anchor, Extensions, Plugin, Services, SettingsPage, StatusAlign, StatusItem, TitleItem, ToolWindow, ViewBuilder,
};
pub use settings::{SettingsChanged, SettingsScope, SettingsStore};
pub use skin::header_controls as tool_window_controls;
pub use theme::{ThemeDef, ThemeKind};
pub use workbench::{EDITOR_AREA, Workbench};

/// Activates `plugins` in order and installs what they contribute.
/// Call once, after `gpui_component::init`, before opening a [`Workbench`].
pub fn activate(plugins: &[Box<dyn Plugin>], cx: &mut gpui::App) {
    plugin::activate(plugins, cx);
}

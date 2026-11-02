//! The editor service (React: `EditorService`): any plugin asks it to open a
//! file; the plugin that provides the editor area answers and reports which
//! file is active, so others (the project tree) can follow it.

use gpui::{Context, EventEmitter};
use std::path::{Path, PathBuf};

pub enum EditorEvent {
    /// A plugin asked to show `path`; the editor area opens it.
    OpenRequested(PathBuf),
    /// The file shown in the editor changed.
    ActiveChanged(Option<PathBuf>),
}

#[derive(Default)]
pub struct Editors {
    active: Option<PathBuf>,
}

impl EventEmitter<EditorEvent> for Editors {}

impl Editors {
    pub fn open(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        cx.emit(EditorEvent::OpenRequested(path));
    }

    pub fn active(&self) -> Option<&Path> {
        self.active.as_deref()
    }

    /// Called by the editor area when it shows another file (or none).
    pub fn set_active(&mut self, path: Option<PathBuf>, cx: &mut Context<Self>) {
        if self.active != path {
            self.active = path.clone();
            cx.emit(EditorEvent::ActiveChanged(path));
            cx.notify();
        }
    }
}

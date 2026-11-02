//! The spike's project view as one plugin: it provides the editor area and the
//! `file.save` action (its keys come from the keymap), built only on
//! `client-core` extension points. Step 5 of wave 1 splits it into a
//! project-tree plugin and core services.

use crate::workspace::Workspace;
use client_core::{ActionDef, Extensions, Plugin};
use gpui::{App, AppContext, WeakEntity};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

pub struct SpikeWorkspace {
    /// Opened once, when the editor area is first built.
    open: Rc<RefCell<Option<PathBuf>>>,
    current: Rc<RefCell<Option<WeakEntity<Workspace>>>>,
}

impl SpikeWorkspace {
    pub fn new(open: Option<PathBuf>) -> Self {
        Self { open: Rc::new(RefCell::new(open)), current: Rc::default() }
    }
}

impl Plugin for SpikeWorkspace {
    fn id(&self) -> &'static str {
        "spike-workspace"
    }

    fn activate(&self, ext: &mut Extensions, _: &mut App) {
        let (open, current) = (self.open.clone(), self.current.clone());
        ext.set_editor_area(Rc::new(move |services, window, cx| {
            let ws = cx.new(|cx| Workspace::new(services, cx));
            if let Some(file) = open.borrow_mut().take() {
                ws.update(cx, |ws, cx| ws.open_file(file, window, cx));
            }
            *current.borrow_mut() = Some(ws.downgrade());
            ws.into()
        }));

        let current = self.current.clone();
        ext.add_action(
            ActionDef::new("file.save", "Save", move |_, _, cx| {
                if let Some(ws) = current.borrow().as_ref().and_then(WeakEntity::upgrade) {
                    ws.update(cx, |ws, cx| ws.save(cx));
                }
            })
            .category("File"),
        );
    }
}

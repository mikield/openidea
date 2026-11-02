//! The editor plugin: provides the workbench's editor area and the
//! `file.save` action, on `client-core` extension points only. Files open
//! when any plugin asks [`client_core::Editors`]; problems come from
//! [`client_core::DiagnosticsStore`].

mod completion;
mod doc_sync;
mod editor_area;

use client_core::{ActionDef, Extensions, Plugin};
use editor_area::EditorArea;
use gpui::{App, AppContext, WeakEntity};
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Default)]
pub struct EditorPlugin {
    current: Rc<RefCell<Option<WeakEntity<EditorArea>>>>,
}

impl Plugin for EditorPlugin {
    fn id(&self) -> &'static str {
        "editor"
    }

    fn activate(&self, ext: &mut Extensions, _: &mut App) {
        let current = self.current.clone();
        ext.set_editor_area(Rc::new(move |services, window, cx| {
            let area = cx.new(|cx| EditorArea::new(services, window, cx));
            *current.borrow_mut() = Some(area.downgrade());
            area.into()
        }));

        let current = self.current.clone();
        ext.add_action(
            ActionDef::new("file.save", "Save", move |_, _, cx| {
                if let Some(area) = current.borrow().as_ref().and_then(WeakEntity::upgrade) {
                    area.update(cx, |area, cx| area.save(cx));
                }
            })
            .category("File"),
        );

        for (id, title, step) in [
            ("editor.closeTab", "Close Tab", 0isize),
            ("editor.nextTab", "Select Next Tab", 1),
            ("editor.prevTab", "Select Previous Tab", -1),
        ] {
            let current = self.current.clone();
            ext.add_action(
                ActionDef::new(id, title, move |_, window, cx| {
                    if let Some(area) = current.borrow().as_ref().and_then(WeakEntity::upgrade) {
                        area.update(cx, |area, cx| match step {
                            0 => area.close_active(window, cx),
                            step => area.cycle(step, window, cx),
                        });
                    }
                })
                .category("Window"),
            );
        }
    }
}

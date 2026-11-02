//! The editor plugin: provides the workbench's editor area and the
//! `file.save` action, on `client-core` extension points only. Files open
//! when any plugin asks [`client_core::Editors`]; problems come from
//! [`client_core::DiagnosticsStore`]. Go to declaration (Ctrl+B, Ctrl+click) and hover
//! come from the language servers; the caret position shows in the status bar. Reformat Code,
//! Rename and Find Usages (with its `usages` tool window) are computed and applied by the server.

pub mod code_insight;
mod completion;
mod doc_sync;
mod editor_area;
mod navigation;
mod usages;

use client_core::{ActionDef, Anchor, Extensions, Plugin, StatusAlign, StatusItem, ToolWindow};
use editor_area::{CaretSink, EditorArea};
use gpui::{App, AppContext, Context, IntoElement, ParentElement, Render, WeakEntity, Window, div};
use std::cell::RefCell;
use std::rc::Rc;
use usages::UsagesView;

#[derive(Default)]
pub struct EditorPlugin {
    current: Rc<RefCell<Option<WeakEntity<EditorArea>>>>,
    caret: Rc<RefCell<Option<WeakEntity<CaretLabel>>>>,
    usages: Rc<RefCell<Option<WeakEntity<UsagesView>>>>,
}

/// The status bar's `line:column` (IDEA's position widget); empty without a document.
struct CaretLabel(Option<(u32, u32)>);

impl Render for CaretLabel {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().child(self.0.map(|(line, col)| format!("{line}:{col}")).unwrap_or_default())
    }
}

impl Plugin for EditorPlugin {
    fn id(&self) -> &'static str {
        "editor"
    }

    fn activate(&self, ext: &mut Extensions, _: &mut App) {
        let caret = self.caret.clone();
        ext.add_status_item(StatusItem {
            id: "caret",
            align: StatusAlign::Right,
            priority: 100,
            build: Rc::new(move |_, _, cx| {
                let label = cx.new(|_| CaretLabel(None));
                *caret.borrow_mut() = Some(label.downgrade());
                label.into()
            }),
        });
        let caret = self.caret.clone();
        let on_caret: CaretSink = Rc::new(move |at, cx| {
            if let Some(label) = caret.borrow().as_ref().and_then(WeakEntity::upgrade) {
                label.update(cx, |label, cx| {
                    if label.0 != at {
                        label.0 = at;
                        cx.notify();
                    }
                });
            }
        });

        let current = self.current.clone();
        ext.set_editor_area(Rc::new(move |services, window, cx| {
            let area = cx.new(|cx| EditorArea::new(services, on_caret.clone(), window, cx));
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

        let current = self.current.clone();
        ext.add_action(
            ActionDef::new("navigate.declaration", "Go to Declaration or Usages", move |_, window, cx| {
                if let Some(area) = current.borrow().as_ref().and_then(WeakEntity::upgrade) {
                    area.update(cx, |area, cx| area.go_to_declaration(window, cx));
                }
            })
            .category("Navigate"),
        );

        let usages = self.usages.clone();
        ext.add_tool_window(ToolWindow {
            id: "usages",
            title: "Find Usages".into(),
            icon: "toolwindow-usages".into(),
            anchor: Anchor::Bottom,
            order: 35,
            own_header: false,
            build: Rc::new(move |services, _, cx| {
                let view = cx.new(|cx| UsagesView::new(services, cx));
                *usages.borrow_mut() = Some(view.downgrade());
                view.into()
            }),
        });

        let (current, usages) = (self.current.clone(), self.usages.clone());
        ext.add_action(
            ActionDef::new("find.usages", "Find Usages", move |workbench, window, cx| {
                let Some(area) = current.borrow().as_ref().and_then(WeakEntity::upgrade) else { return };
                let Some(view) = usages.borrow().as_ref().and_then(WeakEntity::upgrade) else { return };
                let Some((symbol, search)) = area.read(cx).find_usages(cx) else { return };
                view.update(cx, |view, cx| view.search(symbol, search, cx));
                if !workbench.is_tool_window_visible("usages", cx) {
                    workbench.toggle_tool_window("usages", window, cx);
                }
            })
            .category("Find"),
        );

        let current = self.current.clone();
        ext.add_action(
            ActionDef::new("code.reformat", "Reformat Code", move |_, _, cx| {
                if let Some(area) = current.borrow().as_ref().and_then(WeakEntity::upgrade) {
                    area.update(cx, |area, cx| area.reformat(cx));
                }
            })
            .category("Code"),
        );

        let current = self.current.clone();
        ext.add_action(
            ActionDef::new("refactor.rename", "Rename…", move |_, window, cx| {
                if let Some(area) = current.borrow().as_ref().and_then(WeakEntity::upgrade) {
                    area.update(cx, |area, cx| area.start_rename(window, cx));
                }
            })
            .category("Refactor"),
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

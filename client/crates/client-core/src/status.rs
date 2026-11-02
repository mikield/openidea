//! The workbench's own status bar item: what the backend is busy with
//! (indexing, builds), contributed like any plugin's; empty when idle.

use crate::plugin::{Extensions, StatusAlign, StatusItem};
use gpui::{AppContext, Context, IntoElement, ParentElement, Render, SharedString, Window, div};
use ide_core::IdeEvent;
use std::rc::Rc;

pub(crate) fn contribute(ext: &mut Extensions) {
    ext.add_status_item(StatusItem {
        id: "tasks",
        align: StatusAlign::Left,
        priority: 50,
        build: Rc::new(|services, _, cx| {
            let backend = services.backend.clone();
            cx.new(|cx| {
                cx.subscribe(&backend, |this: &mut TaskStatus, _, event: &IdeEvent, cx| {
                    if let IdeEvent::Task { title, message, fraction, done, .. } = event {
                        this.0 = task_text(title, message.as_deref(), *fraction, *done);
                        cx.notify();
                    }
                })
                .detach();
                TaskStatus(SharedString::default())
            })
            .into()
        }),
    });
}

struct TaskStatus(SharedString);

impl Render for TaskStatus {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().child(self.0.clone())
    }
}

fn task_text(title: &str, message: Option<&str>, fraction: Option<f32>, done: bool) -> SharedString {
    // Idle: nothing, like React's status bar.
    if done {
        return SharedString::default();
    }
    let message = message.map(|m| format!(": {m}")).unwrap_or_default();
    let percent = fraction.map(|f| format!(" {:.0}%", f * 100.0)).unwrap_or_default();
    format!("{title}{message}{percent}").into()
}

#[cfg(test)]
mod tests {
    #[test]
    fn task_text_reads_like_the_ide() {
        assert_eq!(super::task_text("Indexing", Some("std"), Some(0.42), false).as_ref(), "Indexing: std 42%");
        assert_eq!(super::task_text("Indexing", None, None, true).as_ref(), "");
    }
}

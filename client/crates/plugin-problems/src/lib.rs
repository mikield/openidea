//! The Problems tool window (`toolwindow.problems`, Alt+6): every diagnostic
//! [`client_core::DiagnosticsStore`] holds, by file. A click opens the file
//! through [`client_core::Editors`].

use client_core::{Anchor, DiagnosticsStore, Editors, Extensions, Plugin, Services, ToolWindow};
use gpui::{
    App, AppContext, ClickEvent, Context, Entity, InteractiveElement, IntoElement, ParentElement, Render,
    StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_component::{ActiveTheme, h_flex};
use ide_core::{Diagnostic, Severity};
use std::path::{Path, PathBuf};
use std::rc::Rc;

pub struct ProblemsPlugin;

impl Plugin for ProblemsPlugin {
    fn id(&self) -> &'static str {
        "problems"
    }

    fn activate(&self, ext: &mut Extensions, _: &mut App) {
        ext.add_tool_window(ToolWindow {
            id: "problems",
            title: "Problems".into(),
            icon: "toolwindow-problems".into(),
            anchor: Anchor::Bottom,
            order: 0,
            own_header: false,
            build: Rc::new(|services, _, cx| cx.new(|cx| Problems::new(services, cx)).into()),
        });
    }
}

pub struct Problems {
    root: PathBuf,
    diagnostics: Entity<DiagnosticsStore>,
    editors: Entity<Editors>,
}

impl Problems {
    pub fn new(services: &Services, cx: &mut Context<Self>) -> Self {
        cx.observe(&services.diagnostics, |_, _, cx| cx.notify()).detach();
        Self {
            root: services.backend.read(cx).project().root.0.clone(),
            diagnostics: services.diagnostics.clone(),
            editors: services.editors.clone(),
        }
    }
}

/// `src/main.rs:3:5` — 1-based, relative to the project root when inside it.
fn location(root: &Path, path: &Path, d: &Diagnostic) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    format!("{}:{}:{}", rel.display(), d.range.start.line + 1, d.range.start.col + 1)
}

impl Render for Problems {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let rows: Vec<_> =
            self.diagnostics.read(cx).problems().all().map(|(path, d)| (path.to_path_buf(), d.clone())).collect();
        if rows.is_empty() {
            return div()
                .size_full()
                .p_2()
                .text_size(px(13.))
                .text_color(theme.muted_foreground)
                .child("No problems")
                .into_any_element();
        }
        div()
            .id("problems")
            .size_full()
            .overflow_y_scroll()
            .py_1()
            .text_size(px(13.))
            .children(rows.into_iter().enumerate().map(|(ix, (path, d))| {
                let color = match d.severity {
                    Severity::Error => theme.danger,
                    Severity::Warning => theme.warning,
                    _ => theme.muted_foreground,
                };
                let editors = self.editors.clone();
                let open = path.clone();
                h_flex()
                    .id(ix)
                    .px_2()
                    .gap_2()
                    .hover(|row| row.bg(theme.list_hover))
                    .child(div().text_color(color).child(format!("{:?}", d.severity)))
                    .child(location(&self.root, &path, &d))
                    .child(
                        div()
                            .text_color(theme.muted_foreground)
                            .child(d.message.lines().next().unwrap_or("").to_owned()),
                    )
                    .on_click(move |_: &ClickEvent, _, cx| editors.update(cx, |e, cx| e.open(open.clone(), cx)))
            }))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ide_core::{Pos, Range};

    #[test]
    fn locations_are_one_based_and_project_relative() {
        let d: Diagnostic = serde_json::from_value(serde_json::json!({
            "range": Range { start: Pos::new(2, 4), end: Pos::new(2, 5) },
            "severity": Severity::Error,
            "message": "x",
        }))
        .unwrap();
        assert_eq!(
            location(Path::new("/p"), Path::new("/p/src/main.rs"), &d),
            format!("{}:3:5", Path::new("src/main.rs").display())
        );
        assert_eq!(
            location(Path::new("/p"), Path::new("/q/x.rs"), &d),
            format!("{}:3:5", Path::new("/q/x.rs").display())
        );
    }
}

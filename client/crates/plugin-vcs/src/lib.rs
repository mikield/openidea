//! The VCS plugin. Wave 3 brings changes, commit and history; for now it shows
//! the current branch in the title bar (React: the VCS title item), refreshed
//! on `VcsChanged`.

use client_core::theme::token;
use client_core::{Extensions, Icon, Plugin, Services, StatusAlign, TitleItem};
use client_rpc::Connection;
use gpui::{App, AppContext, Context, IntoElement, ParentElement, Render, SharedString, Styled, Window, div, px};
use gpui_component::h_flex;
use ide_core::IdeEvent;
use serde::Deserialize;
use serde_json::json;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

pub struct VcsPlugin;

impl Plugin for VcsPlugin {
    fn id(&self) -> &'static str {
        "vcs"
    }

    fn activate(&self, ext: &mut Extensions, _: &mut App) {
        ext.add_title_item(TitleItem {
            id: "vcs-branch",
            align: StatusAlign::Left,
            priority: 50,
            build: Rc::new(|services, _, cx| cx.new(|cx| Branch::new(services, cx)).into()),
        });
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BranchInfo {
    head: Option<String>,
    head_commit: Option<String>,
}

/// The branch label: the branch, a detached HEAD's short commit, or nothing outside a repository.
fn label(info: &BranchInfo) -> Option<String> {
    info.head.clone().or_else(|| info.head_commit.as_ref().map(|c| c.chars().take(8).collect()))
}

struct Branch {
    conn: Arc<Connection>,
    root: PathBuf,
    label: Option<SharedString>,
}

impl Branch {
    fn new(services: &Services, cx: &mut Context<Self>) -> Self {
        cx.subscribe(&services.backend, |this, _, event: &IdeEvent, cx| {
            if let IdeEvent::VcsChanged { .. } = event {
                this.refresh(cx);
            }
        })
        .detach();
        let backend = services.backend.read(cx);
        let mut branch =
            Self { conn: backend.connection().clone(), root: backend.project().root.0.clone(), label: None };
        branch.refresh(cx);
        branch
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let read = self.conn.invoke::<BranchInfo>("vcs_branch_info", json!({ "repo": self.root }));
        cx.spawn(async move |this, cx| {
            // Not a repository (or not trusted yet): no branch to show.
            let label = read.await.ok().as_ref().and_then(label).map(SharedString::from);
            let _ = this.update(cx, |this, cx| {
                this.label = label;
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for Branch {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        match &self.label {
            Some(label) => h_flex()
                .gap(px(4.))
                .text_color(token(cx, "fg"))
                .child(Icon::new("branch").size(px(16.)))
                .child(label.clone())
                .into_any_element(),
            None => div().into_any_element(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_is_the_branch_or_a_detached_commit() {
        let info = |head: Option<&str>, commit: Option<&str>| BranchInfo {
            head: head.map(Into::into),
            head_commit: commit.map(Into::into),
        };
        assert_eq!(label(&info(Some("main"), Some("abc"))).as_deref(), Some("main"));
        assert_eq!(label(&info(None, Some("0123456789ab"))).as_deref(), Some("01234567"));
        assert_eq!(label(&info(None, None)), None);
    }
}

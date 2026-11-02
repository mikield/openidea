//! The workbench in a GPUI test window: default docks, showing and hiding tool
//! windows, and a saved layout restored with a tool window added since.

use crate::layout::LayoutStore;
use crate::{Anchor, Backend, Extensions, Plugin, Services, ToolWindow, ViewBuilder, Workbench};
use client_rpc::proto::{ProjectInfo, ServerInfo};
use client_rpc::{Connection, Transport};
use gpui::{
    App, AppContext, Context, Entity, IntoElement, ParentElement, Render, TestAppContext, VisualTestContext, Window,
    div,
};
use gpui_component::dock::DockPlacement;
use ide_core::IdeResult;
use ide_jsonrpc::{Handler, Peer, codes};
use serde_json::{Value, json};
use std::path::Path;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

/// Answers `initialize` only; enough for a connection that nothing else is asked of.
struct FakeServer(Mutex<Option<Peer>>);

impl Handler for FakeServer {
    fn on_notification(&self, _: &str, _: Value) {}

    fn on_request(&self, method: &str, _: Value) -> Result<Value, (i64, String)> {
        match method {
            "initialize" => Ok(
                json!({ "version": "test", "protocol": client_rpc::proto::PROTOCOL_VERSION, "os": "test", "arch": "test", "home": "/" }),
            ),
            other => Err((codes::METHOD_NOT_FOUND, other.to_owned())),
        }
    }
}

struct InMemory(Arc<FakeServer>);

impl Transport for InMemory {
    fn connect(&self, handler: Arc<dyn Handler>) -> IdeResult<Peer> {
        let (client, server) = tokio::io::duplex(64 * 1024);
        let (read, write) = tokio::io::split(server);
        *self.0.0.lock().unwrap() = Some(Peer::connect(read, write, self.0.clone()));
        let (read, write) = tokio::io::split(client);
        Ok(Peer::connect(read, write, handler))
    }

    fn describe(&self) -> String {
        "in-memory".into()
    }
}

struct Label(&'static str);

impl Render for Label {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().child(self.0)
    }
}

fn label(text: &'static str) -> ViewBuilder {
    Rc::new(move |_, _, cx| cx.new(|_| Label(text)).into())
}

fn tool(id: &'static str, anchor: Anchor, order: i32) -> ToolWindow {
    ToolWindow { id, title: id.into(), anchor, order, build: label(id) }
}

struct TestPlugin(Vec<ToolWindow>);

impl Plugin for TestPlugin {
    fn id(&self) -> &'static str {
        "test"
    }

    fn activate(&self, ext: &mut Extensions, _: &mut App) {
        ext.set_editor_area(label("editor"));
        for tool_window in &self.0 {
            ext.add_tool_window(tool_window.clone());
        }
    }
}

fn services(rt: &tokio::runtime::Runtime, cx: &mut TestAppContext) -> Services {
    let transport = InMemory(Arc::new(FakeServer(Mutex::new(None))));
    let (conn, server, events) = rt.block_on(Connection::connect(&transport, "main")).unwrap();
    let server: ServerInfo = server;
    let project: ProjectInfo =
        serde_json::from_value(json!({ "trusted": true, "root": "/work/a", "name": "a", "isGit": false })).unwrap();
    cx.update(|cx| {
        let backend = cx.new(|cx| Backend::new(conn, rt.handle().clone(), server, project, events, cx));
        Services::new(backend, cx)
    })
}

fn open(services: &Services, layout: &LayoutStore, cx: &mut TestAppContext) -> (Entity<Workbench>, VisualTestContext) {
    let (services, layout) = (services.clone(), layout.clone());
    let (workbench, cx) = cx.add_window_view(|window, cx| Workbench::new(services, layout, window, cx));
    (workbench, cx.clone())
}

fn visible(workbench: &Entity<Workbench>, cx: &mut VisualTestContext) -> Vec<&'static str> {
    ["project", "problems", "terminal", "structure"]
        .into_iter()
        .filter(|id| workbench.read_with(cx, |wb, cx| wb.is_tool_window_visible(id, cx)))
        .collect()
}

fn toggle(workbench: &Entity<Workbench>, id: &str, cx: &mut VisualTestContext) {
    workbench.update_in(cx, |wb, window, cx| wb.toggle_tool_window(id, window, cx));
}

#[gpui::test]
fn tool_windows_dock_toggle_and_their_layout_persists(cx: &mut TestAppContext) {
    // The connection runs on tokio threads, which wake GPUI tasks from outside the test scheduler.
    cx.executor().allow_parking();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let layout = LayoutStore::in_dir(dir.path(), Path::new("/work/a"));
    cx.update(|cx| {
        gpui_component::init(cx);
        let plugins: Vec<Box<dyn Plugin>> = vec![Box::new(TestPlugin(vec![
            tool("terminal", Anchor::Bottom, 1),
            tool("problems", Anchor::Bottom, 0),
            tool("project", Anchor::Left, 0),
        ]))];
        crate::activate(&plugins, cx);
    });
    let services = services(&rt, cx);

    let (workbench, mut vcx) = open(&services, &layout, cx);
    let vcx = &mut vcx;
    // Each anchor's first tool window is the selected tab of its open dock.
    assert_eq!(visible(&workbench, vcx), ["project", "problems"]);
    workbench.read_with(vcx, |wb, cx| {
        let area = wb.dock().read(cx);
        assert!(area.has_dock(DockPlacement::Left) && area.has_dock(DockPlacement::Bottom));
        assert!(!area.has_dock(DockPlacement::Right));
    });

    toggle(&workbench, "terminal", vcx);
    assert_eq!(visible(&workbench, vcx), ["project", "terminal"]);
    toggle(&workbench, "terminal", vcx);
    assert_eq!(visible(&workbench, vcx), ["project"], "showing the shown one hides its dock");
    workbench.update_in(vcx, |wb, window, cx| {
        assert!(wb.run_action("workbench.toggleLeftDock", window, cx));
        wb.save_layout(cx);
    });
    assert_eq!(visible(&workbench, vcx), Vec::<&str>::new());

    // A plugin added a tool window since: the saved layout comes back, the new one joins its anchor.
    cx.update(|cx| {
        let plugins: Vec<Box<dyn Plugin>> = vec![Box::new(TestPlugin(vec![tool("structure", Anchor::Right, 0)]))];
        crate::activate(&plugins, cx);
    });
    let (restored, mut vcx) = open(&services, &layout, cx);
    let vcx = &mut vcx;
    assert_eq!(visible(&restored, vcx), ["structure"]);
    toggle(&restored, "terminal", vcx);
    assert_eq!(visible(&restored, vcx), ["terminal", "structure"], "terminal stayed the bottom dock's tab");
}

//! `openidea-gpui [PROJECT_DIR] [--open RELATIVE_FILE[:LINE:COL]]... [--wait MS] [--run-action ACTION_ID]...`
//!
//! Phase-0 spike: a GPUI client of `oi-server` (upstream's backend plus our
//! commands). Even locally the client spawns the server and speaks the remote
//! protocol, so the same code path serves SSH sessions later. The window is a
//! `client-core` workbench; everything in it comes from the plugins listed here.

use anyhow::Context as _;
use client_core::layout::LayoutStore;
use client_core::{Backend, Plugin, Services, Workbench};
use client_rpc::{Connection, Stdio, Transport as _};
use gpui::{AppContext, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size};
use gpui_component::{Root, TitleBar};
use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_env("OPENIDEA_LOG").unwrap_or_else(|_| "info".into()))
        .init();

    let mut args = std::env::args().skip(1);
    let (mut project, mut open, mut actions, mut wait) = (None, Vec::new(), Vec::new(), 0);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--open" => open.extend(args.next().as_deref().map(file_at)),
            // Delays the actions, e.g. until opened files are loaded and analysed.
            "--wait" => wait = args.next().and_then(|ms| ms.parse().ok()).unwrap_or(0),
            // Runs an action once the window is up (screenshots, demos): `--run-action navigate.gotoFile`.
            "--run-action" => actions.extend(args.next()),
            _ => project = Some(PathBuf::from(arg)),
        }
    }
    let project = project.map_or_else(std::env::current_dir, Ok)?;
    let root = std::fs::canonicalize(&project).with_context(|| project.display().to_string())?;

    // The connection lives on its own runtime; GPUI drives the UI thread.
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build()?;
    let server = Stdio::locate();
    let started = std::time::Instant::now();
    let (conn, info, events, project) = rt
        .block_on(async {
            let (conn, info, events) = Connection::connect(&server, "main").await?;
            // Opening a folder from the command line is the user's trust decision.
            conn.trust_project(&root).await?;
            let project = conn.open_project(&root).await?;
            Ok::<_, ide_core::IdeError>((conn, info, events, project))
        })
        .map_err(|e| anyhow::anyhow!("{}: {}", server.describe(), e.message))?;
    tracing::info!("connected to {} in {:?}", server.describe(), started.elapsed());
    let handle = rt.handle().clone();
    let open: Vec<_> = open.into_iter().map(|(file, at)| (project.root.0.join(file), at)).collect();
    let layout = LayoutStore::for_project(&project.root.0);

    gpui_platform::application().with_assets(client_core::Assets).run(move |cx| {
        gpui_component::init(cx);
        // Keymap and theme follow the settings once the workbench opens.
        let plugins: Vec<Box<dyn Plugin>> = vec![
            Box::new(plugin_presets::Presets),
            Box::new(plugin_editor::EditorPlugin::default()),
            Box::new(plugin_project_tree::ProjectTreePlugin),
            Box::new(plugin_problems::ProblemsPlugin),
            Box::new(plugin_vcs::VcsPlugin),
            Box::new(plugin_terminal::TerminalPlugin),
            Box::new(plugin_search::SearchPlugin),
            Box::new(plugin_menu::MenuPlugin),
        ];
        client_core::activate(&plugins, cx);

        let backend = cx.new(|cx| Backend::new(conn, handle, info, project, events, cx));
        let services = Services::new(backend, cx);
        let editors = services.editors.clone();
        let bounds = Bounds::centered(None, size(px(1280.), px(800.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            // Frameless: the workbench draws the title bar (project, branch, window controls).
            titlebar: Some(TitlebarOptions { title: Some("OpenIDEA · GPUI".into()), ..TitleBar::title_bar_options() }),
            ..Default::default()
        };
        let mut workbench = None;
        let handle = cx
            .open_window(options, |window, cx| {
                let wb = cx.new(|cx| Workbench::new(services, layout, window, cx));
                workbench = Some(wb.clone());
                tracing::info!("window open after {:?}", started.elapsed());
                cx.new(|cx| Root::new(wb, window, cx))
            })
            .expect("open window");
        for (file, at) in open {
            editors.update(cx, |editors, cx| editors.open_at(file, at, cx));
        }
        if let Some(workbench) = workbench {
            cx.spawn(async move |cx| {
                cx.background_executor().timer(std::time::Duration::from_millis(wait)).await;
                for action in actions {
                    let _ = handle
                        .update(cx, |_, window, cx| workbench.update(cx, |wb, cx| wb.run_action(&action, window, cx)));
                }
            })
            .detach();
        }
        cx.activate(true);
    });
    rt.shutdown_timeout(std::time::Duration::from_secs(5));
    Ok(())
}

/// `src/main.rs:6:17` → the file and its 1-based line and column, as a protocol position.
fn file_at(arg: &str) -> (PathBuf, Option<ide_core::Pos>) {
    let mut parts = arg.rsplitn(3, ":");
    if let (Some(col), Some(line), Some(file)) = (parts.next(), parts.next(), parts.next())
        && let (Ok(line), Ok(col)) = (line.parse::<u32>(), col.parse::<u32>())
    {
        return (file.into(), Some(ide_core::Pos::new(line.saturating_sub(1), col.saturating_sub(1))));
    }
    (arg.into(), None)
}

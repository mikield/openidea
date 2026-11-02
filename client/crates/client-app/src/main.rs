//! `openidea-gpui [PROJECT_DIR] [--open RELATIVE_FILE]`
//!
//! Phase-0 spike: a GPUI client of `oi-server` (upstream's backend plus our
//! commands). Even locally the client spawns the server and speaks the remote
//! protocol, so the same code path serves SSH sessions later. The window is a
//! `client-core` workbench; everything in it comes from plugins.

mod completion;
mod doc_sync;
mod spike_plugin;
mod workspace;

use anyhow::Context as _;
use client_core::layout::LayoutStore;
use client_core::{Backend, Plugin, Services, SettingsStore, Workbench};
use client_rpc::{Connection, Stdio, Transport as _};
use gpui::{AppContext, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size};
use gpui_component::{Root, Theme, ThemeMode};
use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_env("OPENIDEA_LOG").unwrap_or_else(|_| "info".into()))
        .init();

    let mut args = std::env::args().skip(1);
    let (mut project, mut open) = (None, None);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--open" => open = args.next().map(PathBuf::from),
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
    let open = open.map(|file| project.root.0.join(file));
    let layout = LayoutStore::for_project(&project.root.0);

    gpui_platform::application().with_assets(gpui_kit_assets::Assets).run(move |cx| {
        gpui_component::init(cx);
        Theme::change(ThemeMode::Dark, None, cx);
        let plugins: Vec<Box<dyn Plugin>> = vec![Box::new(spike_plugin::SpikeWorkspace::new(open))];
        client_core::activate(&plugins, cx);

        let backend = cx.new(|cx| Backend::new(conn, handle, info, project, events, cx));
        let settings = cx.new(|cx| SettingsStore::new(backend.clone(), Default::default(), cx));
        let services = Services { backend, settings };
        let bounds = Bounds::centered(None, size(px(1280.), px(800.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions { title: Some("OpenIDEA · GPUI".into()), ..Default::default() }),
            ..Default::default()
        };
        cx.open_window(options, |window, cx| {
            let workbench = cx.new(|cx| Workbench::new(services, layout, window, cx));
            tracing::info!("window open after {:?}", started.elapsed());
            cx.new(|cx| Root::new(workbench, window, cx))
        })
        .expect("open window");
        cx.activate(true);
    });
    rt.shutdown_timeout(std::time::Duration::from_secs(5));
    Ok(())
}

//! The window's backend as a GPUI entity: commands go through its
//! [`Connection`], server events are re-emitted to every subscriber.

use client_rpc::Connection;
use client_rpc::proto::{ProjectInfo, ServerInfo};
use gpui::{Context, EventEmitter};
use ide_core::IdeEvent;
use std::sync::Arc;
use tokio::runtime::Handle;
use tokio::sync::mpsc::UnboundedReceiver;

pub struct Backend {
    conn: Arc<Connection>,
    rt: Handle,
    server: ServerInfo,
    project: ProjectInfo,
}

impl EventEmitter<IdeEvent> for Backend {}

impl Backend {
    /// `events` is the receiver [`Connection::connect`] returned; subscribe to the
    /// entity (`cx.subscribe(&backend, |_, _, event: &IdeEvent, _| …)`) to get them.
    pub fn new(
        conn: Arc<Connection>,
        rt: Handle,
        server: ServerInfo,
        project: ProjectInfo,
        mut events: UnboundedReceiver<IdeEvent>,
        cx: &mut Context<Self>,
    ) -> Self {
        // tokio's mpsc is executor-agnostic, so GPUI awaits it directly.
        cx.spawn(async move |this, cx| {
            while let Some(event) = events.recv().await {
                if this.update(cx, |_, cx| cx.emit(event)).is_err() {
                    break;
                }
            }
        })
        .detach();
        Self { conn, rt, server, project }
    }

    pub fn connection(&self) -> &Arc<Connection> {
        &self.conn
    }

    /// The runtime the connection runs on, for work that must stay next to it.
    pub fn runtime(&self) -> &Handle {
        &self.rt
    }

    pub fn server(&self) -> &ServerInfo {
        &self.server
    }

    pub fn project(&self) -> &ProjectInfo {
        &self.project
    }
}

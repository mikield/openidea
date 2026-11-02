//! The window's effective settings, kept in step with the backend.
//!
//! Settings live on the server (`settings_get` / `settings_set`, global and
//! per-project layers); this store holds the effective [`Settings`] of the
//! window's project, writes through the server and re-reads on
//! `SettingsChanged`, so every view reads one consistent value.

use crate::Backend;
use gpui::{Context, Entity, EventEmitter, Subscription, Task};
use ide_core::settings::Settings;
use ide_core::{IdeError, IdeEvent, IdeResult};
use serde::Serialize;
use serde_json::{Value, json};

/// Which layer a write goes to; a project value overrides the global one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SettingsScope {
    Global,
    Project,
}

/// Emitted by [`SettingsStore`] after its value changed.
pub struct SettingsChanged;

pub struct SettingsStore {
    backend: Entity<Backend>,
    settings: Settings,
    _events: Subscription,
}

impl EventEmitter<SettingsChanged> for SettingsStore {}

impl SettingsStore {
    /// Starts from `initial` (the defaults until the first read) and reads the server's value.
    pub fn new(backend: Entity<Backend>, initial: Settings, cx: &mut Context<Self>) -> Self {
        let events = cx.subscribe(&backend, |this, _, event: &IdeEvent, cx| {
            if let IdeEvent::SettingsChanged { .. } = event {
                this.reload(cx);
            }
        });
        let mut store = Self { backend, settings: initial, _events: events };
        store.reload(cx);
        store
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Re-reads the effective settings from the server.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let read = self.backend.read(cx).connection().invoke::<Settings>("settings_get", json!({}));
        cx.spawn(async move |this, cx| match read.await {
            Ok(settings) => {
                let _ = this.update(cx, |this, cx| this.replace(settings, cx));
            }
            Err(e) => tracing::warn!("settings_get: {}", e.message),
        })
        .detach();
    }

    /// Writes the dotted `key` (`editor.fontSize`, map entries too) in `scope`;
    /// `null` removes it. Resolves once the server applied it.
    pub fn set(
        &mut self,
        scope: SettingsScope,
        key: &str,
        value: Value,
        cx: &mut Context<Self>,
    ) -> Task<IdeResult<()>> {
        // `settings_set` answers with the effective settings after the write.
        let write = self.backend.read(cx).connection().invoke::<Settings>("settings_set", set_args(scope, key, &value));
        cx.spawn(async move |this, cx| {
            let settings = write.await?;
            this.update(cx, |this, cx| this.replace(settings, cx)).map_err(|e| IdeError::internal(e.to_string()))
        })
    }

    fn replace(&mut self, settings: Settings, cx: &mut Context<Self>) {
        if self.settings != settings {
            self.settings = settings;
            cx.emit(SettingsChanged);
            cx.notify();
        }
    }
}

/// `settings_set` takes the value as JSON text.
fn set_args(scope: SettingsScope, key: &str, value: &Value) -> Value {
    json!({ "scope": scope, "key": key, "value": value.to_string() })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A write through `settings_set` comes back from `settings_get` (a real `oi-server`; skipped unbuilt).
    #[tokio::test(flavor = "multi_thread")]
    async fn settings_round_trip_through_the_backend() {
        let server = client_rpc::Stdio::locate();
        if !server.program.exists() {
            eprintln!("skipped: {} not built", server.program.display());
            return;
        }
        let home = tempfile::tempdir().unwrap();
        let transport = client_rpc::Stdio { program: server.program, home: Some(home.path().into()) };
        let (conn, _, _events) = client_rpc::Connection::connect(&transport, "main").await.unwrap();
        let applied: Settings = conn
            .invoke("settings_set", set_args(SettingsScope::Global, "appearance.theme", &json!("one-dark")))
            .await
            .unwrap();
        assert_eq!(applied.appearance.theme, "one-dark");
        conn.invoke::<Settings>("settings_set", set_args(SettingsScope::Global, "keymap", &json!("vscode-windows")))
            .await
            .unwrap();
        let settings: Settings = conn.invoke("settings_get", json!({})).await.unwrap();
        assert_eq!((settings.appearance.theme.as_str(), settings.keymap.as_str()), ("one-dark", "vscode-windows"));
    }

    #[test]
    fn set_sends_the_value_as_json_text() {
        let args = set_args(SettingsScope::Project, "editor.fontSize", &json!(14));
        assert_eq!(args, json!({ "scope": "project", "key": "editor.fontSize", "value": "14" }));
        let args = set_args(SettingsScope::Global, "keymap", &json!("vscode"));
        assert_eq!(args["value"], "\"vscode\"");
    }
}

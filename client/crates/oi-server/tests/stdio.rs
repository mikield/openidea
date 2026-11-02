//! `oi-server` end to end over stdio, through the client's own `client-rpc`.

use client_rpc::{Connection, Stdio};
use serde_json::{Value, json};
use std::path::PathBuf;

#[tokio::test(flavor = "multi_thread")]
async fn serves_standard_and_own_commands() {
    let home = tempfile::tempdir().unwrap();
    let transport = Stdio { program: PathBuf::from(env!("CARGO_BIN_EXE_oi-server")), home: Some(home.path().into()) };
    let (conn, info, _events) = Connection::connect(&transport, "main").await.expect("start oi-server");
    assert_eq!(info.protocol, client_rpc::proto::PROTOCOL_VERSION);

    let commands = conn.commands().await.unwrap();
    for c in ["open_project", "doc_open", "settings_get", oi_server::commands::SERVER_INFO] {
        assert!(commands.iter().any(|x| x == c), "oi-server lacks {c}");
    }

    // Upstream's trust gate covers our commands too: outside its allowlist they need a
    // trusted project open in the window.
    let refused = conn.invoke::<Value>(oi_server::commands::SERVER_INFO, json!({})).await;
    assert!(refused.is_err(), "ran without a project: {refused:?}");
    let project = tempfile::tempdir().unwrap();
    conn.trust_project(project.path()).await.unwrap();
    conn.open_project(project.path()).await.unwrap();
    let own: Value = conn.invoke(oi_server::commands::SERVER_INFO, json!({})).await.unwrap();
    assert_eq!(own["name"], "oi-server");
}

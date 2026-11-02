//! The backend's search commands answer in the shapes the popup reads.
//! Needs `oi-server` built (`cargo build -p oi-server`); skipped otherwise.

use client_rpc::{Connection, Stdio};
use serde_json::{Value, json};
use std::time::Duration;

#[tokio::test(flavor = "multi_thread")]
async fn find_files_and_symbols_answer_in_the_popup_shapes() {
    let server = Stdio::locate();
    if !server.program.exists() {
        eprintln!("skipped: {} not built", server.program.display());
        return;
    }
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let root = dunce(project.path());
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/greeter.rs"), "pub struct Greeter;\npub fn say_hello() {}\n").unwrap();
    let transport = Stdio { program: server.program, home: Some(home.path().into()) };
    let (conn, _, _events) = Connection::connect(&transport, "main").await.expect("start oi-server");
    conn.trust_project(&root).await.unwrap();
    conn.open_project(&root).await.unwrap();

    // The index fills in the background: poll.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    let (mut files, mut symbols) = (Vec::<Value>::new(), Vec::<Value>::new());
    while tokio::time::Instant::now() < deadline && (files.is_empty() || symbols.is_empty()) {
        files = conn.invoke("find_files", json!({ "query": "greet", "limit": 10 })).await.unwrap();
        symbols =
            conn.invoke("find_symbols", json!({ "query": "Greeter", "filter": "types", "limit": 10 })).await.unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    let file = files.first().expect("find_files found nothing");
    assert!(file["relPath"].as_str().unwrap().ends_with("greeter.rs"), "{file}");
    assert!(file["path"].is_string());
    let symbol = symbols.first().expect("find_symbols found nothing");
    assert_eq!(symbol["name"], "Greeter");
    assert!(symbol["kind"].is_string() && symbol["relPath"].is_string(), "{symbol}");
    let _ = conn.invoke::<Value>("recent_files", json!({})).await.expect("recent_files");
}

/// The server reports paths without Windows' `\\?\` prefix.
fn dunce(p: &std::path::Path) -> std::path::PathBuf {
    let path = std::fs::canonicalize(p).unwrap();
    match path.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
        Some(plain) => plain.into(),
        None => path,
    }
}

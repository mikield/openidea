//! End-to-end against the real, unmodified `openidea-server`.
//!
//! Needs the server binary: `cargo build -p ide-host --bin openidea-server` at the
//! repository root, or `OPENIDEA_SERVER=<path>`. Skipped (with a note) when absent.

use client_rpc::{Connection, ServerCommand, text};
use ide_core::{ChangeSet, IdeEvent, Pos, Severity};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::mpsc::UnboundedReceiver;

fn server() -> Option<PathBuf> {
    let path = std::env::var_os("OPENIDEA_SERVER")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../target/debug/openidea-server"));
    if path.exists() {
        Some(path)
    } else {
        eprintln!("skipped: {} not built", path.display());
        None
    }
}

fn fixture(dir: &Path, main_rs: &str) {
    std::fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n",
    )
    .unwrap();
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/main.rs"), main_rs).unwrap();
}

async fn connect(home: &Path, server: PathBuf) -> (std::sync::Arc<Connection>, UnboundedReceiver<IdeEvent>) {
    let cmd = ServerCommand { program: server, home: Some(home.to_path_buf()) };
    let (conn, info, events) = Connection::spawn(&cmd, "main").await.expect("spawn server");
    assert_eq!(info.protocol, 2);
    (conn, events)
}

#[tokio::test(flavor = "multi_thread")]
async fn project_tree_and_document_round_trip() {
    let Some(server) = server() else { return };
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let root = dunce_canonical(project.path());
    fixture(&root, "fn main() {\n    println!(\"привіт\");\n}\n");
    let (conn, _events) = connect(home.path(), server).await;

    let commands = conn.commands().await.unwrap();
    for c in ["open_project", "fs_read_dir", "doc_open", "doc_edit", "doc_save", "lsp_completion"] {
        assert!(commands.iter().any(|x| x == c), "server lacks {c}");
    }

    conn.trust_project(&root).await.unwrap();
    let info = conn.open_project(&root).await.unwrap();
    assert!(info.trusted);
    assert_eq!(info.root.0, root);

    let entries = conn.read_dir(&root).await.unwrap();
    let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
    assert!(names.contains(&"Cargo.toml") && names.contains(&"src"), "{names:?}");
    assert!(entries.iter().find(|e| e.name == "src").unwrap().is_dir());

    let main = root.join("src/main.rs");
    let doc = conn.doc_open(&main).await.unwrap();
    assert_eq!(doc.language, "rust");
    assert!(!doc.dirty);

    // Edit after the Cyrillic text: offsets must be UTF-16 on the wire.
    let new_text = doc.text.replace("привіт", "привіт, світ");
    let change = text::diff(&doc.text, &new_text).unwrap();
    let v = conn.doc_edit(doc.id, doc.version, ChangeSet { changes: vec![change] }).await.unwrap();
    assert!(v > doc.version);
    conn.doc_save(doc.id).await.unwrap();
    assert_eq!(std::fs::read_to_string(&main).unwrap(), new_text);

    // A stale base version is refused, not silently applied.
    let stale = conn.doc_edit(doc.id, doc.version, ChangeSet { changes: vec![text::diff("", "x").unwrap()] }).await;
    assert!(stale.is_err(), "stale edit applied: {stale:?}");
    assert_eq!(std::fs::read_to_string(&main).unwrap(), new_text);
}

/// Language server through the protocol: diagnostics events and completion.
/// Needs `rust-analyzer` on `PATH`.
#[tokio::test(flavor = "multi_thread")]
async fn rust_analyzer_diagnostics_and_completion() {
    let Some(server) = server() else { return };
    if std::process::Command::new("rust-analyzer").arg("--version").output().is_err() {
        eprintln!("skipped: rust-analyzer not installed");
        return;
    }
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let root = dunce_canonical(project.path());
    let src = "fn main() {\n    let n: i32 = \"not a number\";\n    let v: Vec<i32> = Vec::new();\n    v.\n}\n";
    fixture(&root, src);
    let (conn, mut events) = connect(home.path(), server).await;
    conn.trust_project(&root).await.unwrap();
    conn.open_project(&root).await.unwrap();
    let main = root.join("src/main.rs");
    let doc = conn.doc_open(&main).await.unwrap();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
    let error = loop {
        let ev = tokio::time::timeout_at(deadline, events.recv()).await.expect("no diagnostics in time").unwrap();
        if let IdeEvent::Diagnostics { path, items, .. } = ev
            && path.0 == main
            // `v.` is a syntax error reported at once; wait for the type error on line 1,
            // which needs semantic analysis.
            && let Some(e) = items.into_iter().find(|d| d.severity == Severity::Error && d.range.start.line == 1)
        {
            break e;
        }
    };
    assert!(error.message.contains("mismatched types"), "{error:?}");

    // `v.` → Vec methods. Retry while the server is still indexing.
    let pos = text::byte_to_pos(&doc.text, doc.text.find("v.\n").unwrap() + 2);
    assert_eq!(pos, Pos::new(3, 6));
    let mut labels = Vec::new();
    for _ in 0..30 {
        let list = conn.completion(doc.id, pos, Some(".".into())).await.unwrap();
        labels = list.items.into_iter().map(|i| i.label).collect();
        if labels.iter().any(|l| l.starts_with("push")) {
            break;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    assert!(labels.iter().any(|l| l.starts_with("push")), "{labels:?}");
}

/// The server reports canonical paths (`/tmp` may be a symlink).
fn dunce_canonical(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap()
}

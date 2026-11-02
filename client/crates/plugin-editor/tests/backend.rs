//! The editor's backend contract against a real `oi-server` (skipped when it is not built):
//! a `DocPatched` merged with `text::apply` gives the server's text, and go to / hover
//! answer in the shapes `navigation.rs` reads, and Find Usages, Rename and Reformat Code
//! work end to end (with `rust-analyzer` on `PATH`).

use client_rpc::{Connection, Stdio, text};
use ide_core::{FileEdit, IdeEvent, Pos, Range, TextEdit, WorkspaceEdit};
use plugin_editor::code_insight;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::Duration;

const MAIN: &str = "fn greet() -> i32 {\n    1\n}\n\nfn main() {\n    let _x = greet();\n}\n";

fn plain(p: &Path) -> PathBuf {
    let path = std::fs::canonicalize(p).unwrap();
    match path.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
        Some(s) => s.into(),
        None => path,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn patches_merge_and_navigation_answers() {
    let server = Stdio::locate();
    if !server.program.exists() {
        eprintln!("skipped: {} not built", server.program.display());
        return;
    }
    let (home, project) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let root = plain(project.path());
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"fx\"\nversion = \"0.1.0\"\nedition = \"2021\"\n")
        .unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    let main = root.join("src/main.rs");
    std::fs::write(&main, MAIN).unwrap();
    let transport = Stdio { program: server.program, home: Some(home.path().into()) };
    let (conn, _, mut events) = Connection::connect(&transport, "main").await.expect("start oi-server");
    conn.trust_project(&root).await.unwrap();
    conn.open_project(&root).await.unwrap();
    let doc = conn.doc_open(&main).await.unwrap();

    // The backend edits the document: the event's changes, applied our way, give its text.
    let edit = WorkspaceEdit {
        files: vec![FileEdit::Edit {
            path: main.clone().into(),
            edits: vec![TextEdit {
                range: Range { start: Pos::new(5, 8), end: Pos::new(5, 10) },
                new_text: "_y".into(),
            }],
        }],
    };
    conn.invoke::<Value>("apply_workspace_edit", json!({ "edit": edit })).await.unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let (base, version, changes) = loop {
        match tokio::time::timeout_at(deadline, events.recv()).await.expect("no DocPatched").unwrap() {
            IdeEvent::DocPatched { doc: d, base_version, version, changes } if d == doc.id => {
                break (base_version, version, changes);
            }
            _ => {}
        }
    };
    assert_eq!(base, doc.version);
    assert!(version > base);
    let mut merged = doc.text.clone();
    text::apply(&mut merged, &changes);
    conn.doc_save(doc.id).await.unwrap();
    assert_eq!(std::fs::read_to_string(&main).unwrap(), merged);
    // Columns 8..10 (`_x`) became `_y`.
    assert!(merged.contains("    let _y = greet();"), "{merged}");

    if !std::process::Command::new("rust-analyzer").arg("--version").output().is_ok_and(|o| o.status.success()) {
        eprintln!("skipped navigation: rust-analyzer not installed");
        return;
    }
    // `greet()` in main: line 5, column 16 is inside `greet`.
    let at = Pos::new(5, 16);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
    let targets = loop {
        let targets: Vec<Value> =
            match conn.invoke("lsp_goto", json!({ "doc": doc.id, "pos": at, "kind": "definition" })).await {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("lsp_goto: {}", e.message);
                    Vec::new()
                }
            };
        if !targets.is_empty() || tokio::time::Instant::now() > deadline {
            break targets;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    };
    let target = targets.first().expect("no definition in time");
    assert_eq!(target["range"]["start"]["line"], 0, "{target}");
    assert!(target["path"].as_str().unwrap().ends_with("main.rs"), "{target}");
    // rust-analyzer cancels requests while it still analyses (`ContentModified`): retry.
    let mut hover = Value::Null;
    for _ in 0..40 {
        let request = json!({ "request": ide_core::RequestId::next(), "doc": doc.id, "pos": at });
        if let Ok(h) = conn.invoke::<Value>("lsp_hover", request).await
            && !h.is_null()
        {
            hover = h;
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    assert!(hover["contents"]["value"].as_str().unwrap_or_default().contains("greet"), "{hover}");

    // Find Usages from the declaration: the call in `main`, with its line.
    let mut found = Default::default();
    for _ in 0..40 {
        if let Ok(r) = code_insight::find_usages(conn.clone(), doc.id, Pos::new(0, 4)).await
            && r.count() > 0
        {
            found = r;
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let code_insight::UsageResults { files, .. } = found;
    let (path, usages) = files.first().expect("no usages in time");
    assert!(path.ends_with("main.rs"), "{}", path.display());
    assert_eq!(usages.len(), 1, "{usages:?}");
    assert_eq!(usages[0].range.start, Pos::new(5, 13));
    assert!(usages[0].line.contains("greet();"), "{:?}", usages[0].line);

    // Rename: the server's range is the name; the edit lands in the document.
    let range = code_insight::prepare_rename(conn.clone(), doc.id, merged.clone(), Pos::new(0, 4)).await.unwrap();
    assert_eq!(range.map(|r| &merged[r]), Some("greet"));
    let (occurrences, files) =
        code_insight::rename(conn.clone(), doc.id, Pos::new(0, 4), "hello".into()).await.unwrap();
    assert_eq!((occurrences, files), (2, 1));
    conn.doc_save(doc.id).await.unwrap();
    let saved = std::fs::read_to_string(&main).unwrap();
    assert!(saved.contains("fn hello()") && saved.contains("= hello();"), "{saved}");

    // Reformat: an over-indented body comes back with four spaces.
    let indent = WorkspaceEdit {
        files: vec![FileEdit::Edit {
            path: main.clone().into(),
            edits: vec![TextEdit {
                range: Range { start: Pos::new(1, 0), end: Pos::new(1, 0) },
                new_text: "      ".into(),
            }],
        }],
    };
    conn.invoke::<Value>("apply_workspace_edit", json!({ "edit": indent })).await.unwrap();
    let changed = code_insight::reformat(conn.clone(), doc.id, main.clone(), String::new()).await.unwrap();
    assert!(changed > 0);
    conn.doc_save(doc.id).await.unwrap();
    let saved = std::fs::read_to_string(&main).unwrap();
    assert!(saved.contains("{\n    1\n}"), "{saved}");
}

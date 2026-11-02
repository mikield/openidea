//! One terminal session: a shell on a backend PTY (`term_spawn`), its output
//! fed to the [`Emulator`], keystrokes written back. Spawn, writes and resizes
//! go through one ordered queue, so keys never overtake each other or the spawn.

use crate::emulator::{ANSI_TOKENS, Emulator, Palette, encode};
use alacritty_terminal::term::TermMode;
use client_core::Services;
use client_core::theme::token;
use client_rpc::Connection;
use gpui::{
    App, Context, FocusHandle, Focusable, FontStyle, FontWeight, InteractiveElement, IntoElement, KeyDownEvent,
    MouseButton, ParentElement, Pixels, Render, ScrollEvent, SharedString, Styled, StyledText, TextRun, UnderlineStyle,
    Window, canvas, div, font, px,
};
use gpui_component::ActiveTheme;
use ide_core::IdeEvent;
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

const FONT_SIZE: f32 = 13.;
const LINE_HEIGHT: f32 = 17.;

enum Op {
    Spawn { cols: u16, rows: u16, channel: Value, info: oneshot::Sender<Option<Info>> },
    Write(String),
    Resize(u16, u16),
    Kill,
}

#[derive(Clone, Deserialize)]
pub struct Info {
    id: Value,
}

/// Runs the queue on the connection's runtime, one request at a time.
async fn run(conn: Arc<Connection>, cwd: PathBuf, mut ops: mpsc::UnboundedReceiver<Op>) {
    let mut id = Value::Null;
    while let Some(op) = ops.recv().await {
        let result = match op {
            Op::Spawn { cols, rows, channel, info } => {
                let options = json!({ "cwd": cwd, "shell": null, "args": [], "env": {}, "cols": cols, "rows": rows });
                let spawned =
                    conn.invoke::<Info>("term_spawn", json!({ "options": options, "onOutput": channel })).await;
                if let Ok(spawned) = &spawned {
                    id = spawned.id.clone();
                }
                let _ = info.send(spawned.as_ref().ok().cloned());
                spawned.map(|_| Value::Null)
            }
            Op::Write(data) => conn.invoke::<Value>("term_write", json!({ "id": id, "data": data })).await,
            Op::Resize(cols, rows) => {
                conn.invoke::<Value>("term_resize", json!({ "id": id, "cols": cols, "rows": rows })).await
            }
            Op::Kill => conn.invoke::<Value>("term_kill", json!({ "id": id })).await,
        };
        if let Err(e) = result {
            tracing::warn!("terminal: {}", e.message);
        }
    }
}

pub struct TerminalView {
    emulator: Emulator,
    ops: mpsc::UnboundedSender<Op>,
    conn: Arc<Connection>,
    id: Option<Value>,
    started: bool,
    pub title: SharedString,
    exited: Option<Option<i32>>,
    focus: FocusHandle,
}

impl TerminalView {
    /// `title`: the tab name ("Local", "Local (2)"; React names tabs, not the shell's window title).
    pub fn new(services: &Services, title: SharedString, cx: &mut Context<Self>) -> Self {
        let backend = services.backend.read(cx);
        let (conn, root) = (backend.connection().clone(), backend.project().root.0.clone());
        let (ops, rx) = mpsc::unbounded_channel();
        backend.runtime().spawn(run(conn.clone(), root, rx));
        cx.subscribe(&services.backend, |this, _, event: &IdeEvent, cx| {
            if let IdeEvent::TerminalExited { terminal, code } = event
                && this.id.as_ref() == serde_json::to_value(terminal).ok().as_ref()
            {
                this.exited = Some(*code);
                cx.notify();
            }
        })
        .detach();
        Self {
            emulator: Emulator::new(80, 24),
            ops,
            conn,
            id: None,
            started: false,
            title,
            exited: None,
            focus: cx.focus_handle(),
        }
    }

    fn send(&self, op: Op) {
        let _ = self.ops.send(op);
    }

    /// The grid that fits the view; the shell starts once it is known.
    fn set_grid(&mut self, cols: usize, rows: usize, cx: &mut Context<Self>) {
        if self.started && self.emulator.size() == (cols, rows) {
            return;
        }
        self.emulator.resize(cols, rows);
        let (cols, rows) = (cols as u16, rows as u16);
        if self.started {
            self.send(Op::Resize(cols, rows));
        } else {
            self.started = true;
            self.start(cols, rows, cx);
        }
        cx.notify();
    }

    fn start(&mut self, cols: u16, rows: u16, cx: &mut Context<Self>) {
        let (channel, mut output) = self.conn.open_channel();
        let (info_tx, info_rx) = oneshot::channel();
        self.send(Op::Spawn { cols, rows, channel, info: info_tx });
        cx.spawn(async move |this, cx| {
            if let Ok(Some(info)) = info_rx.await {
                let _ = this.update(cx, |view, cx| {
                    view.id = Some(info.id);
                    cx.notify();
                });
            }
            while let Some(chunk) = output.recv().await {
                let Some(text) = chunk.as_str() else { continue };
                let alive = this.update(cx, |view, cx| {
                    let effects = view.emulator.feed(text.as_bytes());
                    if !effects.reply.is_empty() {
                        view.send(Op::Write(effects.reply));
                    }
                    cx.notify();
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn write(&mut self, text: String) {
        // Typing returns to the live screen from the scrollback.
        self.emulator.scroll(i32::MIN / 2);
        self.send(Op::Write(text));
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let key = &event.keystroke;
        let paste = (key.modifiers.control && key.modifiers.shift && key.key == "v")
            || (key.modifiers.shift && key.key == "insert");
        if paste {
            if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                let bracketed = self.emulator.mode().contains(TermMode::BRACKETED_PASTE);
                self.write(if bracketed { format!("\x1b[200~{text}\x1b[201~") } else { text });
            }
        } else if let Some(bytes) = encode(key, self.emulator.mode().contains(TermMode::APP_CURSOR)) {
            self.write(bytes);
        } else {
            return;
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn on_scroll(&mut self, event: &ScrollEvent, _: &mut Window, cx: &mut Context<Self>) {
        let lines = (event.delta.pixel_delta(px(LINE_HEIGHT)).y / px(LINE_HEIGHT)).round() as i32;
        if lines != 0 {
            self.emulator.scroll(lines);
            cx.notify();
        }
    }
}

impl Drop for TerminalView {
    fn drop(&mut self) {
        if self.started {
            let _ = self.ops.send(Op::Kill);
        }
    }
}

impl Focusable for TerminalView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let family = cx.theme().mono_font_family.clone();
        let mono = font(family.clone());
        let size = px(FONT_SIZE);
        let line = px(LINE_HEIGHT);
        let text_system = window.text_system();
        let cell_width: Pixels =
            text_system.advance(text_system.resolve_font(&mono), size, 'm').map(|s| s.width).unwrap_or(px(8.));
        let palette =
            Palette { ansi: ANSI_TOKENS.map(|t| token(cx, t)), fg: token(cx, "fg"), bg: token(cx, "bg-island") };
        let (lines, cursor) = self.emulator.screen();
        let rows = lines.into_iter().map(|spans| {
            let mut text = String::new();
            let runs: Vec<TextRun> = spans
                .iter()
                .map(|span| {
                    text.push_str(&span.text);
                    let mut run_font = mono.clone();
                    if span.bold {
                        run_font.weight = FontWeight::BOLD;
                    }
                    if span.italic {
                        run_font.style = FontStyle::Italic;
                    }
                    TextRun {
                        len: span.text.len(),
                        font: run_font,
                        color: palette.resolve(&span.fg, false).unwrap_or(palette.fg),
                        background_color: palette.resolve(&span.bg, true),
                        underline: span.underline.then(|| UnderlineStyle {
                            thickness: px(1.),
                            color: None,
                            wavy: false,
                        }),
                        strikethrough: None,
                    }
                })
                .collect();
            div().h(line).flex_none().whitespace_nowrap().child(StyledText::new(text).with_runs(runs))
        });
        let focused = self.focus.is_focused(window);
        let caret = token(cx, "caret");
        let cursor = cursor.filter(|_| self.exited.is_none()).map(|(col, row)| {
            div()
                .absolute()
                .left(cell_width * col as f32)
                .top(line * row as f32)
                .w(cell_width)
                .h(line)
                .border_1()
                .border_color(caret)
                .when(focused, |c| c.bg(caret.opacity(0.6)))
        });
        let exited = self.exited.map(|code| {
            let code = code.map(|c| format!(" with code {c}")).unwrap_or_default();
            div().text_color(token(cx, "fg-muted")).child(format!("[Process exited{code}]"))
        });
        let view = cx.entity().downgrade();
        let sizer = canvas(
            move |bounds, _, cx| {
                let cols = (bounds.size.width / cell_width).floor().max(2.) as usize;
                let rows = (bounds.size.height / line).floor().max(1.) as usize;
                let _ = view.update(cx, |view, cx| view.set_grid(cols, rows, cx));
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();
        div()
            .id("terminal")
            .track_focus(&self.focus)
            .size_full()
            .p(px(6.))
            .font_family(family)
            .text_size(size)
            .line_height(line)
            .text_color(palette.fg)
            .on_key_down(cx.listener(Self::on_key_down))
            .on_scroll(cx.listener(Self::on_scroll))
            .on_mouse_down(MouseButton::Left, cx.listener(|view, _, window, cx| view.focus.focus(window, cx)))
            .child(
                div()
                    .relative()
                    .size_full()
                    .overflow_hidden()
                    .child(sizer)
                    .children(rows)
                    .children(cursor)
                    .children(exited),
            )
    }
}

use gpui::prelude::FluentBuilder as _;

#[cfg(test)]
mod tests {
    use super::*;
    use client_rpc::Stdio;
    use std::time::Duration;

    /// A real shell on a real `oi-server` PTY, through the same queue the view uses.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_shell_runs_on_the_backend_pty() {
        let server = Stdio::locate();
        if !server.program.exists() {
            eprintln!("skipped: {} not built", server.program.display());
            return;
        }
        let home = tempfile::tempdir().unwrap();
        let cwd = tempfile::tempdir().unwrap();
        let transport = Stdio { program: server.program, home: Some(home.path().into()) };
        let (conn, _, _events) = Connection::connect(&transport, "main").await.expect("start oi-server");
        let (ops, rx) = mpsc::unbounded_channel();
        tokio::spawn(run(conn.clone(), cwd.path().into(), rx));
        let (channel, mut output) = conn.open_channel();
        let (info, info_rx) = oneshot::channel();
        ops.send(Op::Spawn { cols: 100, rows: 30, channel, info }).unwrap();
        assert!(info_rx.await.unwrap().is_some(), "term_spawn failed");

        let mut emulator = Emulator::new(100, 30);
        let screen = |emulator: &Emulator| -> String {
            emulator
                .screen()
                .0
                .iter()
                .map(|line| line.iter().map(|s| s.text.as_str()).collect::<String>() + "\n")
                .collect()
        };
        let mut sent = false;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            let chunk = tokio::time::timeout_at(deadline, output.recv())
                .await
                .unwrap_or_else(|_| panic!("no echo in time; screen:\n{}", screen(&emulator)))
                .expect("output closed");
            let effects = emulator.feed(chunk.as_str().unwrap_or_default().as_bytes());
            // Windows' conpty waits for the answer to its cursor position query.
            if !effects.reply.is_empty() {
                ops.send(Op::Write(effects.reply)).unwrap();
            }
            if !sent {
                ops.send(Op::Write("echo oi-term-42\r".into())).unwrap();
                sent = true;
            }
            // The command line and its output: the marker twice.
            if screen(&emulator).matches("oi-term-42").count() >= 2 {
                break;
            }
        }
        ops.send(Op::Kill).unwrap();
    }
}

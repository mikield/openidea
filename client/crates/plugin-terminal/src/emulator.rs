//! The terminal emulator: `alacritty_terminal` parses the PTY's output into a
//! grid; this module turns the grid into styled lines and keystrokes into the
//! bytes a shell expects. No GPUI windows involved, so it is testable as is.

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::{Color, NamedColor, Processor, Rgb};
use gpui::{Hsla, Keystroke, Rgba};
use std::sync::{Arc, Mutex};

/// Collects what the terminal asks of its host (answers to queries, a new title).
#[derive(Clone, Default)]
pub struct Listener(Arc<Mutex<Vec<Event>>>);

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        if let Ok(mut events) = self.0.lock() {
            events.push(event);
        }
    }
}

/// What the host must do after feeding output.
#[derive(Debug, Default, PartialEq)]
pub struct Effects {
    /// Bytes to write back to the PTY (replies to device status queries…).
    pub reply: String,
    pub title: Option<String>,
}

/// A run of cells sharing one style.
#[derive(Debug, Clone, PartialEq)]
pub struct Span {
    pub text: String,
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

pub struct Emulator {
    term: Term<Listener>,
    parser: Processor,
    listener: Listener,
}

impl Emulator {
    pub fn new(cols: usize, rows: usize) -> Self {
        let listener = Listener::default();
        let config = Config { scrolling_history: 10_000, ..Config::default() };
        let term = Term::new(config, &TermSize::new(cols.max(2), rows.max(1)), listener.clone());
        Self { term, parser: Processor::new(), listener }
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Effects {
        self.parser.advance(&mut self.term, bytes);
        let mut effects = Effects::default();
        for event in self.listener.0.lock().map(|mut e| std::mem::take(&mut *e)).unwrap_or_default() {
            match event {
                Event::PtyWrite(text) => effects.reply.push_str(&text),
                Event::Title(title) => effects.title = Some(title),
                Event::ResetTitle => effects.title = Some(String::new()),
                _ => {}
            }
        }
        effects
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        self.term.resize(TermSize::new(cols.max(2), rows.max(1)));
    }

    pub fn size(&self) -> (usize, usize) {
        (self.term.columns(), self.term.screen_lines())
    }

    pub fn scroll(&mut self, lines: i32) {
        self.term.scroll_display(Scroll::Delta(lines));
    }

    pub fn mode(&self) -> TermMode {
        *self.term.mode()
    }

    /// The visible screen as rows of styled spans, and the cursor (column, row) when on screen.
    pub fn screen(&self) -> (Vec<Vec<Span>>, Option<(usize, usize)>) {
        let content = self.term.renderable_content();
        let offset = content.display_offset as i32;
        let rows = self.term.screen_lines();
        let mut lines: Vec<Vec<Span>> = vec![Vec::new(); rows];
        for indexed in content.display_iter {
            let row = indexed.point.line.0 + offset;
            if row < 0 || row as usize >= rows {
                continue;
            }
            let cell = indexed.cell;
            if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                continue;
            }
            let (mut fg, mut bg) = (cell.fg, cell.bg);
            if cell.flags.contains(Flags::INVERSE) {
                std::mem::swap(&mut fg, &mut bg);
            }
            let span = Span {
                text: cell.c.to_string(),
                fg,
                bg,
                bold: cell.flags.contains(Flags::BOLD),
                italic: cell.flags.contains(Flags::ITALIC),
                underline: cell.flags.intersects(Flags::ALL_UNDERLINES),
            };
            let line = &mut lines[row as usize];
            match line.last_mut() {
                Some(last)
                    if (&last.fg, &last.bg, last.bold, last.italic, last.underline)
                        == (&span.fg, &span.bg, span.bold, span.italic, span.underline) =>
                {
                    last.text.push(cell.c)
                }
                _ => line.push(span),
            }
        }
        let cursor = content.cursor.point;
        let row = cursor.line.0 + offset;
        let shown = self.term.mode().contains(TermMode::SHOW_CURSOR) && row >= 0 && (row as usize) < rows;
        (lines, shown.then_some((cursor.column.0, row as usize)))
    }
}

/// The 16 ANSI colours, from the theme's `terminal-ansi-*` tokens.
pub const ANSI_TOKENS: [&str; 16] = [
    "terminal-ansi-black",
    "terminal-ansi-red",
    "terminal-ansi-green",
    "terminal-ansi-yellow",
    "terminal-ansi-blue",
    "terminal-ansi-magenta",
    "terminal-ansi-cyan",
    "terminal-ansi-white",
    "terminal-ansi-bright-black",
    "terminal-ansi-bright-red",
    "terminal-ansi-bright-green",
    "terminal-ansi-bright-yellow",
    "terminal-ansi-bright-blue",
    "terminal-ansi-bright-magenta",
    "terminal-ansi-bright-cyan",
    "terminal-ansi-bright-white",
];

/// The theme colours a screen is painted with.
pub struct Palette {
    pub ansi: [Hsla; 16],
    pub fg: Hsla,
    pub bg: Hsla,
}

fn rgb(r: u8, g: u8, b: u8) -> Hsla {
    Rgba { r: r as f32 / 255., g: g as f32 / 255., b: b as f32 / 255., a: 1. }.into()
}

/// xterm's 256-colour palette above the 16 ANSI ones: a 6×6×6 cube, then greys.
fn indexed(ix: u8) -> Hsla {
    match ix {
        16..=231 => {
            let i = ix - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            rgb(level(i / 36), level(i / 6 % 6), level(i % 6))
        }
        _ => {
            let v = 8 + (ix.saturating_sub(232)) * 10;
            rgb(v, v, v)
        }
    }
}

impl Palette {
    /// `None` for the default background (the island shows through).
    pub fn resolve(&self, color: &Color, background: bool) -> Option<Hsla> {
        Some(match color {
            Color::Spec(Rgb { r, g, b }) => rgb(*r, *g, *b),
            Color::Indexed(ix) if (*ix as usize) < 16 => self.ansi[*ix as usize],
            Color::Indexed(ix) => indexed(*ix),
            Color::Named(named) => match named {
                NamedColor::Background if background => return None,
                NamedColor::Foreground | NamedColor::BrightForeground | NamedColor::Cursor => self.fg,
                NamedColor::DimForeground => self.fg.opacity(0.7),
                NamedColor::Background => self.bg,
                other => {
                    let ix = *other as usize;
                    if ix < 16 {
                        self.ansi[ix]
                    } else {
                        self.ansi[(ix - NamedColor::DimBlack as usize) % 8].opacity(0.7)
                    }
                }
            },
        })
    }
}

/// The bytes a keystroke sends to the shell, or `None` when it is not for the terminal.
/// `app_cursor`: the shell asked for application cursor keys (DECCKM).
pub fn encode(key: &Keystroke, app_cursor: bool) -> Option<String> {
    let m = &key.modifiers;
    let csi = |s: &str| format!("\x1b[{s}");
    let named = match key.key.as_str() {
        "enter" => Some("\r".to_owned()),
        "backspace" => Some(if m.control { "\x08" } else { "\x7f" }.to_owned()),
        "tab" if m.shift => Some(csi("Z")),
        "tab" => Some("\t".to_owned()),
        "escape" => Some("\x1b".to_owned()),
        "up" | "down" | "right" | "left" => {
            let code = match key.key.as_str() {
                "up" => 'A',
                "down" => 'B',
                "right" => 'C',
                _ => 'D',
            };
            Some(if m.control {
                csi(&format!("1;5{code}"))
            } else if app_cursor {
                format!("\x1bO{code}")
            } else {
                csi(&code.to_string())
            })
        }
        "home" => Some(csi("H")),
        "end" => Some(csi("F")),
        "pageup" => Some(csi("5~")),
        "pagedown" => Some(csi("6~")),
        "delete" => Some(csi("3~")),
        "insert" => Some(csi("2~")),
        "f1" => Some("\x1bOP".to_owned()),
        "f2" => Some("\x1bOQ".to_owned()),
        "f3" => Some("\x1bOR".to_owned()),
        "f4" => Some("\x1bOS".to_owned()),
        f if f.starts_with('f') && f.len() > 1 => {
            let n: u8 = f[1..].parse().ok()?;
            let code = [15, 17, 18, 19, 20, 21, 23, 24].get(n.checked_sub(5)? as usize)?;
            Some(csi(&format!("{code}~")))
        }
        _ => None,
    };
    if named.is_some() {
        return named;
    }
    // Ctrl+letter: the control character (Ctrl+C = ETX…).
    if m.control && !m.alt && key.key.len() == 1 {
        let c = key.key.chars().next()?.to_ascii_lowercase();
        return match c {
            'a'..='z' => Some(((c as u8 - b'a' + 1) as char).to_string()),
            ' ' | '2' | '@' => Some("\0".into()),
            '[' => Some("\x1b".into()),
            '\\' => Some("\x1c".into()),
            ']' => Some("\x1d".into()),
            _ => None,
        };
    }
    let text = key.key_char.clone().or_else(|| (key.key == "space").then(|| " ".to_owned()))?;
    // Alt+key: ESC prefix (meta sends escape), as terminals do.
    Some(if m.alt && !m.control { format!("\x1b{text}") } else { text })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(source: &str, key_char: Option<&str>) -> Keystroke {
        let mut k = Keystroke::parse_with_os(source, gpui::OperatingSystem::Linux).unwrap();
        k.key_char = key_char.map(Into::into);
        k
    }

    #[test]
    fn output_becomes_styled_lines_and_a_cursor() {
        let mut emu = Emulator::new(20, 3);
        let effects = emu.feed(b"hi \x1b[31mred\x1b[0m\r\nnext");
        assert_eq!(effects, Effects::default());
        let (lines, cursor) = emu.screen();
        let first: String = lines[0].iter().map(|s| s.text.as_str()).collect();
        assert!(first.starts_with("hi red"), "{first:?}");
        let red = lines[0].iter().find(|s| s.text.starts_with("red")).unwrap();
        assert_eq!(red.fg, Color::Named(NamedColor::Red));
        assert_eq!(cursor, Some((4, 1)));
    }

    #[test]
    fn device_queries_are_answered_through_the_pty() {
        let mut emu = Emulator::new(20, 3);
        // DSR 6: where is the cursor? (Windows' conpty asks this at start.)
        let effects = emu.feed(b"\x1b[6n\x1b]0;my title\x07");
        assert_eq!(effects.reply, "\x1b[1;1R");
        assert_eq!(effects.title.as_deref(), Some("my title"));
    }

    #[test]
    fn keys_encode_like_a_terminal() {
        assert_eq!(encode(&key("enter", None), false).as_deref(), Some("\r"));
        assert_eq!(encode(&key("ctrl-c", None), false).as_deref(), Some("\x03"));
        assert_eq!(encode(&key("up", None), false).as_deref(), Some("\x1b[A"));
        assert_eq!(encode(&key("up", None), true).as_deref(), Some("\x1bOA"));
        assert_eq!(encode(&key("ctrl-left", None), false).as_deref(), Some("\x1b[1;5D"));
        assert_eq!(encode(&key("f5", None), false).as_deref(), Some("\x1b[15~"));
        assert_eq!(encode(&key("a", Some("a")), false).as_deref(), Some("a"));
        assert_eq!(encode(&key("alt-b", Some("b")), false).as_deref(), Some("\x1bb"));
        assert_eq!(encode(&key("shift", None), false), None);
    }

    #[test]
    fn colours_resolve_through_the_palette() {
        let p = Palette { ansi: [rgb(1, 1, 1); 16], fg: rgb(2, 2, 2), bg: rgb(3, 3, 3) };
        assert_eq!(p.resolve(&Color::Named(NamedColor::Background), true), None);
        assert_eq!(p.resolve(&Color::Named(NamedColor::Foreground), false), Some(rgb(2, 2, 2)));
        assert_eq!(p.resolve(&Color::Indexed(196), false), Some(rgb(255, 0, 0)));
        assert_eq!(p.resolve(&Color::Indexed(232), false), Some(rgb(8, 8, 8)));
    }
}

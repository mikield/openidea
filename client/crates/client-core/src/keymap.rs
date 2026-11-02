//! Keymaps as data: presets (action id → bindings) contributed by plugins,
//! chosen and overridden by settings, bound to [`RunAction`] in GPUI.
//!
//! Bindings use the React keymap spelling (`src/platform/keymap.ts`):
//! `Ctrl+Shift+N`, chords space-separated (`Ctrl+K Ctrl+S`), `Meta` for ⌘.
//! Two kinds have no GPUI equivalent yet and are skipped: double-tapped
//! modifiers (`Shift Shift`) and mouse buttons (`Button4`, `Ctrl+Alt+Button1`).

use crate::RunAction;
use gpui::{App, KeyBinding, KeyBindingContextPredicate, SharedString};
use ide_core::settings::Settings;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::rc::Rc;

/// Action id → bindings; an empty list unbinds.
pub type Bindings = BTreeMap<String, Vec<String>>;

/// A bundled keymap (`intellij-windows`, `vscode-macos`…).
#[derive(Debug, Clone, Deserialize)]
pub struct KeymapPreset {
    pub id: SharedString,
    pub title: SharedString,
    pub bindings: Bindings,
}

/// The GPUI key context our bindings live in.
pub const KEY_CONTEXT: &str = "Workbench";

/// The preset used when settings name none (or an unknown one): IDEA's keymap for this OS.
pub fn default_preset() -> &'static str {
    if cfg!(target_os = "macos") { "intellij-macos" } else { "intellij-windows" }
}

/// The effective bindings for `settings` (React's `applyKeymapSettings`): the
/// chosen preset, or a user keymap's parent preset with that keymap's own
/// overrides, else the settings' `keymapOverrides`.
pub fn effective(presets: &[KeymapPreset], settings: &Settings) -> Bindings {
    let find = |id: &str| presets.iter().find(|p| p.id == id);
    let (preset, overrides) = match settings.custom_keymaps.get(&settings.keymap) {
        Some(custom) if find(&settings.keymap).is_none() => (custom.parent.as_str(), &custom.overrides),
        _ => (settings.keymap.as_str(), &settings.keymap_overrides),
    };
    let base = find(preset).or_else(|| find(default_preset())).or_else(|| presets.first());
    let mut out = base.map(|p| p.bindings.clone()).unwrap_or_default();
    for (action, keys) in overrides {
        out.insert(action.clone(), keys.clone());
    }
    out
}

/// One binding in GPUI's keystroke syntax (`ctrl-shift-n`, `ctrl-k ctrl-s`), or
/// `None` for the kinds GPUI cannot express.
pub fn to_gpui(binding: &str) -> Option<String> {
    let strokes: Vec<&str> = binding.split_whitespace().collect();
    let modifier = |s: &str| matches!(s.to_ascii_lowercase().as_str(), "ctrl" | "alt" | "shift" | "meta");
    if strokes.is_empty() || strokes.iter().any(|s| modifier(s)) {
        // `Shift Shift`, `Ctrl Ctrl+A`: double-tapped modifiers.
        return None;
    }
    strokes.iter().map(|s| stroke(s)).collect::<Option<Vec<_>>>().map(|s| s.join(" "))
}

fn stroke(stroke: &str) -> Option<String> {
    // The key itself may be `+` (`Ctrl++`).
    let (mods, key) = match stroke.strip_suffix("++") {
        Some(mods) => (mods, "+"),
        None => stroke.rsplit_once('+').unwrap_or(("", stroke)),
    };
    let mut out = String::new();
    for m in mods.split('+').filter(|m| !m.is_empty()) {
        out.push_str(match m.to_ascii_lowercase().as_str() {
            "ctrl" => "ctrl-",
            "alt" => "alt-",
            "shift" => "shift-",
            "meta" => "cmd-",
            _ => return None,
        });
    }
    out.push_str(&key_name(key)?);
    Some(out)
}

fn key_name(key: &str) -> Option<String> {
    let lower = key.to_ascii_lowercase();
    if lower.starts_with("button") {
        return None;
    }
    let named = match lower.as_str() {
        "left" | "right" | "up" | "down" | "home" | "end" | "pageup" | "pagedown" | "enter" | "escape"
        | "backspace" | "delete" | "insert" | "tab" | "space" => lower.as_str(),
        "numpadadd" => "+",
        "numpadsubtract" => "-",
        "numpadmultiply" => "*",
        "numpaddivide" => "/",
        _ if lower.len() > 1 && lower.starts_with('f') && lower[1..].parse::<u8>().is_ok() => lower.as_str(),
        _ if key.chars().count() == 1 => lower.as_str(),
        _ => return None,
    };
    Some(named.to_owned())
}

/// Replaces the keymap's [`RunAction`] bindings with `bindings`, keeping every
/// other binding (the components' own). Only ids `bound` accepts are bound, so a
/// key no plugin implements still reaches the focused view (Tab, Enter…).
pub fn apply(bindings: &Bindings, bound: impl Fn(&str) -> bool, cx: &mut App) {
    let predicate = KeyBindingContextPredicate::parse(KEY_CONTEXT).ok().map(Rc::new);
    let mut ours = Vec::new();
    for (id, keys) in bindings.iter().filter(|(id, _)| bound(id)) {
        for key in keys {
            let Some(keystrokes) = to_gpui(key) else { continue };
            let action = Box::new(RunAction { id: id.clone().into() });
            match KeyBinding::load(&keystrokes, action, predicate.clone(), false, None, cx) {
                Ok(binding) => ours.push(binding),
                Err(e) => tracing::warn!("binding {key} ({keystrokes}) for {id}: {e}"),
            }
        }
    }
    let keymap = cx.key_bindings();
    let kept: Vec<KeyBinding> =
        keymap.borrow().bindings().filter(|b| !b.action().as_any().is::<RunAction>()).cloned().collect();
    cx.clear_key_bindings();
    cx.bind_keys(kept.into_iter().chain(ours));
}

#[cfg(test)]
mod tests {
    use super::*;
    use ide_core::settings::CustomKeymap;

    fn preset(id: &str, bindings: &[(&str, &[&str])]) -> KeymapPreset {
        KeymapPreset {
            id: id.to_owned().into(),
            title: id.to_owned().into(),
            bindings: bindings
                .iter()
                .map(|(a, k)| (a.to_string(), k.iter().map(|s| s.to_string()).collect()))
                .collect(),
        }
    }

    #[test]
    fn bindings_translate_to_gpui_keystrokes() {
        assert_eq!(to_gpui("Ctrl+Shift+N").as_deref(), Some("ctrl-shift-n"));
        assert_eq!(to_gpui("Meta+Alt+L").as_deref(), Some("cmd-alt-l"));
        assert_eq!(to_gpui("Ctrl+K Ctrl+S").as_deref(), Some("ctrl-k ctrl-s"));
        assert_eq!(to_gpui("Alt+F12").as_deref(), Some("alt-f12"));
        assert_eq!(to_gpui("Ctrl+Alt+PageDown").as_deref(), Some("ctrl-alt-pagedown"));
        assert_eq!(to_gpui("Ctrl++").as_deref(), Some("ctrl-+"));
        assert_eq!(to_gpui("Ctrl+NumpadAdd").as_deref(), Some("ctrl-+"));
        assert_eq!(to_gpui("Ctrl+Alt+]").as_deref(), Some("ctrl-alt-]"));
        assert_eq!(to_gpui("Shift Shift"), None);
        assert_eq!(to_gpui("Button4"), None);
        assert_eq!(to_gpui("Ctrl+Alt+Button1"), None);
    }

    #[test]
    fn effective_keymap_follows_react_semantics() {
        let presets = [
            preset("intellij-windows", &[("file.save", &["Ctrl+S"]), ("navigate.back", &["Ctrl+Alt+Left"])]),
            preset("intellij-macos", &[("file.save", &["Meta+S"])]),
            preset("vscode-windows", &[("file.save", &["Ctrl+S"]), ("navigate.back", &["Alt+Left"])]),
        ];
        let mut settings = Settings { keymap: "vscode-windows".into(), ..Default::default() };
        settings.keymap_overrides.insert("file.save".into(), vec![]);
        let keys = effective(&presets, &settings);
        assert_eq!(keys["navigate.back"], ["Alt+Left"]);
        assert!(keys["file.save"].is_empty(), "an empty override unbinds");

        // A user keymap: its parent and its own overrides; the global overrides do not apply.
        settings.keymap = "mine".into();
        settings.custom_keymaps.insert(
            "mine".into(),
            CustomKeymap {
                name: "Mine".into(),
                parent: "intellij-windows".into(),
                overrides: [("navigate.back".to_owned(), vec!["F1".to_owned()])].into(),
            },
        );
        let keys = effective(&presets, &settings);
        assert_eq!(
            (keys["file.save"].as_slice(), keys["navigate.back"].as_slice()),
            (&["Ctrl+S".to_owned()][..], &["F1".to_owned()][..])
        );

        settings.keymap = "nonexistent".into();
        settings.keymap_overrides.clear();
        assert_eq!(
            effective(&presets, &settings)["file.save"],
            [if cfg!(target_os = "macos") { "Meta+S" } else { "Ctrl+S" }]
        );
    }
}

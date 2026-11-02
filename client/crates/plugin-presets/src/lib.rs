//! Built-in keymaps (IntelliJ, VS Code, Eclipse; Windows and macOS spellings)
//! and themes (Islands Light / Dark, High Contrast, Solarized, One), as one
//! plugin contributing data.
//!
//! `data/` is generated from the React frontend by
//! `node client/scripts/export-presets.ts`; never edit it by hand. Each file
//! records the upstream revision it was exported from (`from`).

mod pages;

use client_core::{Extensions, KeymapPreset, Plugin, SettingsPage, ThemeDef};
use gpui::{App, AppContext, SharedString};
use pages::{Choice, ChoicePage, current_keymap, current_theme};
use serde::de::DeserializeOwned;
use std::rc::Rc;

macro_rules! data {
    ($($path:literal),* $(,)?) => {
        [$(($path, include_str!(concat!("../data/", $path)))),*]
    };
}

const KEYMAPS: [(&str, &str); 6] = data![
    "keymaps/intellij-windows.json",
    "keymaps/intellij-macos.json",
    "keymaps/vscode-windows.json",
    "keymaps/vscode-macos.json",
    "keymaps/eclipse-windows.json",
    "keymaps/eclipse-macos.json",
];

/// In React's menu order (`src/platform/themes.ts`).
const THEMES: [(&str, &str); 7] = data![
    "themes/light.json",
    "themes/dark.json",
    "themes/dark-hc.json",
    "themes/solarized-light.json",
    "themes/solarized-dark.json",
    "themes/one-light.json",
    "themes/one-dark.json",
];

fn parse<T: DeserializeOwned>(files: &[(&str, &str)]) -> Vec<T> {
    files
        .iter()
        .filter_map(|(path, text)| serde_json::from_str(text).inspect_err(|e| tracing::error!("{path}: {e}")).ok())
        .collect()
}

pub fn keymaps() -> Vec<KeymapPreset> {
    parse(&KEYMAPS)
}

pub fn themes() -> Vec<ThemeDef> {
    parse(&THEMES)
}

pub struct Presets;

impl Plugin for Presets {
    fn id(&self) -> &'static str {
        "presets"
    }

    fn activate(&self, ext: &mut Extensions, _: &mut App) {
        let (keymaps, themes) = (keymaps(), themes());
        let theme_options: Vec<(SharedString, SharedString)> =
            themes.iter().map(|t| (t.id.clone(), t.name.clone())).collect();
        let keymap_options: Vec<(SharedString, SharedString)> =
            keymaps.iter().map(|k| (k.id.clone(), k.title.clone())).collect();
        for preset in keymaps {
            ext.add_keymap(preset);
        }
        for theme in themes {
            ext.add_theme(theme);
        }
        // React: Settings › Appearance & Behavior › Appearance, and Keymap.
        ext.add_settings_page(SettingsPage {
            id: "appearance",
            title: "Appearance".into(),
            group: Some("Appearance & Behavior".into()),
            order: 0,
            keywords: vec!["theme".into(), "dark".into(), "light".into()],
            project_scope: false,
            build: Rc::new(move |services, _, cx| {
                let choice = Choice {
                    key: "appearance.theme",
                    label: "Theme",
                    options: theme_options.clone(),
                    current: current_theme,
                };
                let sync = (
                    "appearance.syncWithOs",
                    "Sync with OS",
                    (|s: &ide_core::settings::Settings| s.appearance.sync_with_os) as fn(&_) -> bool,
                );
                cx.new(|cx| ChoicePage::new(services, choice, Some(sync), cx)).into()
            }),
        });
        ext.add_settings_page(SettingsPage {
            id: "keymap",
            title: "Keymap".into(),
            group: None,
            order: 10,
            keywords: vec!["shortcut".into(), "keyboard".into()],
            project_scope: false,
            build: Rc::new(move |services, _, cx| {
                let choice =
                    Choice { key: "keymap", label: "Keymap", options: keymap_options.clone(), current: current_keymap };
                cx.new(|cx| ChoicePage::new(services, choice, None, cx)).into()
            }),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use client_core::keymap::to_gpui;
    use client_core::theme::theme_config;

    #[test]
    fn every_file_parses() {
        assert_eq!(keymaps().len(), KEYMAPS.len());
        assert_eq!(themes().len(), THEMES.len());
    }

    /// Every binding converts, or is one of the kinds GPUI cannot express yet.
    #[test]
    fn bindings_convert_to_valid_gpui_keystrokes() {
        for preset in keymaps() {
            for (action, keys) in &preset.bindings {
                for key in keys {
                    // Mouse buttons and double-tapped modifiers (`Shift Shift`).
                    let unsupported = key.contains("Button")
                        || key.split_whitespace().any(|s| ["Ctrl", "Alt", "Shift", "Meta"].contains(&s));
                    match to_gpui(key) {
                        Some(gpui) => {
                            for stroke in gpui.split(' ') {
                                gpui::Keystroke::parse_with_os(stroke, gpui::OperatingSystem::Linux)
                                    .unwrap_or_else(|e| panic!("{}: {action} = {key} → {stroke}: {e}", preset.id));
                            }
                        }
                        None => assert!(unsupported, "{}: {action} = {key} did not convert", preset.id),
                    }
                }
            }
        }
    }

    /// The tokens every theme must define for the mapping onto gpui-component.
    #[test]
    fn every_theme_maps_its_core_tokens() {
        for theme in themes() {
            for token in ["bg-island", "bg-editor", "fg", "fg-editor", "accent", "border", "syn-keyword", "syn-string"]
            {
                let value = theme.token(token).unwrap_or_else(|| panic!("{}: no {token}", theme.id));
                assert!(value.starts_with('#'), "{}: {token} = {value}", theme.id);
            }
            let config = serde_json::to_value(theme_config(&theme, None, &Default::default())).unwrap();
            assert_eq!(config["name"], theme.name.as_ref());
            assert!(config["colors"]["background"].is_string(), "{}", theme.id);
            assert!(!config["highlight"]["syntax"]["keyword"]["color"].is_null(), "{}", theme.id);
        }
    }
}

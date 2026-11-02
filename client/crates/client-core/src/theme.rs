//! Themes as data: a theme is the React design tokens (`src/styles/tokens.css`)
//! resolved to values, contributed by plugins. This module picks one from the
//! appearance settings (React: `src/workbench/theme.ts`) and maps its tokens
//! onto gpui-component's theme colors and editor highlight style.

use gpui::{App, SharedString, Window};
use gpui_component::{Theme, ThemeConfig};
use ide_core::settings::AppearanceSettings;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::rc::Rc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeKind {
    Light,
    Dark,
}

/// A theme: token name (without `--`) → resolved value (`#rrggbb[aa]` for colours).
#[derive(Debug, Clone, Deserialize)]
pub struct ThemeDef {
    pub id: SharedString,
    pub name: SharedString,
    pub kind: ThemeKind,
    pub tokens: BTreeMap<String, String>,
}

impl ThemeDef {
    pub fn token(&self, name: &str) -> Option<&str> {
        self.tokens.get(name).map(String::as_str)
    }
}

/// PyCharm's brand blue and the pre-Islands accent: values that mean "the theme's own accent".
const THEME_ACCENTS: [&str; 2] = ["#3871e1", "#8fa5d6"];

/// The theme id for `appearance` (React's `resolveThemeId`): the chosen theme, or
/// with "Sync with OS" (legacy `theme = "system"`) the preferred light / dark one.
/// Unknown ids fall back to Islands Light / Dark.
pub fn resolve_theme_id<'a>(
    appearance: &'a AppearanceSettings,
    system_light: bool,
    known: impl Fn(&str) -> bool,
) -> &'a str {
    let concrete = |id: &'a str, fallback: &'a str| if known(id) { id } else { fallback };
    if appearance.sync_with_os || appearance.theme == "system" {
        if system_light {
            concrete(&appearance.preferred_light_theme, "light")
        } else {
            concrete(&appearance.preferred_dark_theme, "dark")
        }
    } else {
        concrete(&appearance.theme, "light")
    }
}

/// gpui-component colour key ← token. Keys follow `gpui_component::ThemeConfigColors`.
const COLORS: &[(&str, &str)] = &[
    ("background", "bg-island"),
    ("foreground", "fg"),
    ("border", "border"),
    ("accent.background", "bg-hover"),
    ("accent.foreground", "fg"),
    ("muted.background", "bg-elev"),
    ("muted.foreground", "fg-muted"),
    ("primary.background", "accent"),
    ("primary.hover.background", "accent-hover"),
    ("primary.active.background", "accent-hover"),
    ("primary.foreground", "fg-on-accent"),
    ("secondary.background", "bg-button"),
    ("secondary.hover.background", "bg-control-hover"),
    ("secondary.active.background", "bg-active"),
    ("secondary.foreground", "fg"),
    ("button.primary.background", "bg-button-primary"),
    ("button.primary.hover.background", "bg-button-primary-hover"),
    ("button.primary.active.background", "bg-button-primary-hover"),
    ("button.primary.foreground", "fg-button-primary"),
    ("danger.background", "danger"),
    ("danger.foreground", "fg-on-accent"),
    ("warning.background", "warning"),
    ("success.background", "success"),
    ("info.background", "info"),
    ("input.border", "border-strong"),
    ("ring", "focus-ring"),
    ("caret", "caret"),
    ("link", "link"),
    ("link.hover", "link-hover"),
    ("link.active", "link-hover"),
    ("selection.background", "bg-editor-selection"),
    ("overlay", "bg-scrim"),
    ("popover.background", "bg-popup"),
    ("popover.foreground", "fg"),
    ("list.background", "bg-island"),
    ("list.even.background", "bg-island"),
    ("list.head.background", "bg-elev"),
    ("list.hover.background", "bg-tree-hover"),
    ("list.active.background", "bg-tree-selected"),
    ("list.active.border", "bg-tree-selected"),
    ("table.background", "bg-island"),
    ("table.even.background", "bg-island"),
    ("table.head.background", "bg-elev"),
    ("table.head.foreground", "fg-secondary"),
    ("table.hover.background", "bg-tree-hover"),
    ("table.active.background", "bg-tree-selected"),
    ("table.active.border", "bg-tree-selected"),
    ("table.row.border", "border"),
    ("sidebar.background", "bg-island"),
    ("sidebar.foreground", "fg"),
    ("sidebar.border", "border"),
    ("sidebar.accent.background", "bg-hover"),
    ("sidebar.accent.foreground", "fg"),
    ("sidebar.primary.background", "accent"),
    ("sidebar.primary.foreground", "fg-on-accent"),
    ("tab_bar.background", "bg-island"),
    ("tab.background", "bg-island"),
    ("tab.foreground", "fg-secondary"),
    ("tab.active.background", "bg-tab-selected"),
    ("tab.active.foreground", "fg-strong"),
    ("title_bar.background", "bg-frame"),
    ("title_bar.border", "border"),
    ("status_bar.background", "bg-frame"),
    ("status_bar.border", "border"),
    ("window.border", "border-window"),
    ("scrollbar.thumb.background", "scrollbar-thumb"),
    ("scrollbar.thumb.hover.background", "scrollbar-thumb-hover"),
    ("drag.border", "accent"),
    ("drop_target.background", "accent-soft"),
    ("progress.bar.background", "accent"),
    ("skeleton.background", "bg-elev"),
    ("slider.background", "accent"),
    ("slider.thumb.background", "bg-toggle-knob"),
    ("switch.background", "bg-toggle-on"),
    ("switch.thumb.background", "bg-toggle-knob"),
    ("chart.1", "chart-series"),
    ("base.red", "ansi-red"),
    ("base.red.light", "ansi-bright-red"),
    ("base.green", "ansi-green"),
    ("base.green.light", "ansi-bright-green"),
    ("base.yellow", "ansi-yellow"),
    ("base.yellow.light", "ansi-bright-yellow"),
    ("base.blue", "ansi-blue"),
    ("base.blue.light", "ansi-bright-blue"),
    ("base.magenta", "ansi-magenta"),
    ("base.magenta.light", "ansi-bright-magenta"),
    ("base.cyan", "ansi-cyan"),
    ("base.cyan.light", "ansi-bright-cyan"),
];

/// Editor highlight style key ← token (`gpui_component::highlighter::HighlightThemeStyle`).
const EDITOR: &[(&str, &str)] = &[
    ("editor.background", "bg-editor"),
    ("editor.foreground", "fg-editor"),
    ("editor.active_line.background", "bg-current-line"),
    ("editor.line_number", "fg-line-number"),
    ("editor.active_line_number", "fg-line-number-current"),
    ("editor.invisible", "fg-whitespace"),
    ("editor.gutter.background", "bg-editor-gutter"),
    ("error", "danger"),
    ("error.background", "bg-feedback-error"),
    ("error.border", "border-feedback-error"),
    ("warning", "warning"),
    ("warning.background", "bg-feedback-warning"),
    ("warning.border", "border-feedback-warning"),
    ("info", "info"),
    ("info.background", "bg-feedback-info"),
    ("info.border", "border-feedback-info"),
    ("success", "success"),
    ("success.background", "bg-feedback-success"),
    ("success.border", "border-feedback-success"),
    ("hint", "fg-inlay"),
];

/// Tree-sitter capture ← syntax token (the Islands editor scheme's roles).
const SYNTAX: &[(&str, &str)] = &[
    ("attribute", "syn-attribute"),
    ("boolean", "syn-keyword"),
    ("comment", "syn-comment"),
    ("comment.doc", "syn-doc-comment"),
    ("constant", "syn-constant"),
    ("constructor", "syn-type"),
    ("enum", "syn-type"),
    ("function", "syn-function"),
    ("hint", "fg-inlay"),
    ("keyword", "syn-keyword"),
    ("label", "syn-annotation"),
    ("link_text", "syn-link"),
    ("link_uri", "syn-link"),
    ("number", "syn-number"),
    ("operator", "syn-punct"),
    ("preproc", "syn-decorator"),
    ("property", "syn-property"),
    ("punctuation", "syn-punct"),
    ("punctuation.bracket", "syn-punct"),
    ("punctuation.delimiter", "syn-punct"),
    ("punctuation.special", "syn-escape"),
    ("string", "syn-string"),
    ("string.escape", "syn-escape"),
    ("string.regex", "syn-regexp"),
    ("string.special", "syn-string"),
    ("string.special.symbol", "syn-constant"),
    ("tag", "syn-tag"),
    ("text.code.span", "syn-markup-code"),
    ("text.literal", "syn-string"),
    ("title", "syn-markup-heading"),
    ("type", "syn-type"),
    ("variable", "syn-variable"),
    ("variable.special", "syn-self"),
    ("variant", "syn-constant"),
];

fn is_color(value: &str) -> bool {
    value.starts_with('#') && matches!(value.len(), 4 | 5 | 7 | 9) && value[1..].chars().all(|c| c.is_ascii_hexdigit())
}

/// gpui-component's config for `theme`, with `accent` (`#rrggbb`) replacing the
/// theme's own unless it is one of the default accents.
pub fn theme_config(theme: &ThemeDef, accent: Option<&str>) -> ThemeConfig {
    let custom = accent.filter(|a| is_color(a) && !THEME_ACCENTS.contains(&a.to_ascii_lowercase().as_str()));
    let color = |token: &str| -> Option<&str> {
        match (token, custom) {
            ("accent", Some(accent)) => Some(accent),
            _ => theme.token(token).filter(|v| is_color(v)),
        }
    };
    let pick = |table: &[(&str, &str)]| -> Map<String, Value> {
        table.iter().filter_map(|(key, token)| Some((key.to_string(), json!(color(token)?)))).collect()
    };
    let mut highlight = pick(EDITOR);
    // gpui-component built without `tree-sitter` (our `syntect` build) names these
    // `editor_background`…; with it, `editor.background`. Unknown keys are ignored: write both.
    for (key, value) in highlight.clone() {
        if let Some(rest) = key.strip_prefix("editor.").filter(|r| *r != "gutter.background") {
            highlight.insert(format!("editor_{}", rest.replace('.', "_").replace("_background", "")), value);
        }
    }
    let syntax: Map<String, Value> = SYNTAX
        .iter()
        .filter_map(|(capture, token)| {
            let mut style = json!({ "color": color(token)? });
            if *capture == "comment" && theme.token("syn-comment-style") == Some("italic") {
                style["font_style"] = json!("italic");
            }
            Some((capture.to_string(), style))
        })
        .collect();
    highlight.insert("syntax".into(), Value::Object(syntax));
    let config = json!({
        "name": theme.name.as_ref(),
        "mode": match theme.kind { ThemeKind::Dark => "dark", ThemeKind::Light => "light" },
        "colors": pick(COLORS),
        "highlight": highlight,
    });
    serde_json::from_value(config).unwrap_or_else(|e| {
        tracing::warn!("theme {}: {e}", theme.id);
        ThemeConfig::default()
    })
}

/// The current theme, for what gpui-component's colours do not cover (the Islands frame,
/// tool window stripes): read through [`token`].
struct Current(Rc<ThemeDef>);

impl gpui::Global for Current {}

/// A colour token of the current theme (`bg-frame`, `bg-stripe-selected`…); transparent when unknown.
pub fn token(cx: &App, name: &str) -> gpui::Hsla {
    cx.try_global::<Current>()
        .and_then(|t| t.0.token(name))
        .and_then(|v| gpui::Rgba::try_from(v).ok())
        .map(gpui::Hsla::from)
        .unwrap_or_else(gpui::transparent_black)
}

/// Whether the current theme is dark (no theme yet counts as dark, like Islands Dark's `:root`).
pub fn is_dark(cx: &App) -> bool {
    cx.try_global::<Current>().is_none_or(|t| t.0.kind == ThemeKind::Dark)
}

/// Makes `theme` the current one.
pub fn apply(theme: &ThemeDef, accent: Option<&str>, window: Option<&mut Window>, cx: &mut App) {
    let mut current = theme.clone();
    if let Some(accent) = accent.filter(|a| is_color(a) && !THEME_ACCENTS.contains(&a.to_ascii_lowercase().as_str())) {
        current.tokens.insert("accent".into(), accent.to_owned());
    }
    cx.set_global(Current(Rc::new(current)));
    let config = theme_config(theme, accent);
    let mode = config.mode;
    Theme::global_mut(cx).apply_config(&Rc::new(config));
    Theme::change(mode, window, cx);
}

/// Whether the OS is in light mode, for "Sync with OS".
pub fn system_light(window: &Window) -> bool {
    matches!(window.appearance(), gpui::WindowAppearance::Light | gpui::WindowAppearance::VibrantLight)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theme(kind: ThemeKind, tokens: &[(&str, &str)]) -> ThemeDef {
        ThemeDef {
            id: "t".into(),
            name: "T".into(),
            kind,
            tokens: tokens.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        }
    }

    #[test]
    fn theme_id_follows_the_appearance_settings() {
        let known = |id: &str| ["light", "dark", "one-dark", "solarized-light"].contains(&id);
        let mut a = AppearanceSettings { theme: "one-dark".into(), ..Default::default() };
        assert_eq!(resolve_theme_id(&a, true, known), "one-dark");
        a.theme = "nope".into();
        assert_eq!(resolve_theme_id(&a, false, known), "light");
        a.sync_with_os = true;
        a.preferred_light_theme = "solarized-light".into();
        a.preferred_dark_theme = "nope".into();
        assert_eq!(resolve_theme_id(&a, true, known), "solarized-light");
        assert_eq!(resolve_theme_id(&a, false, known), "dark");
    }

    #[test]
    fn tokens_map_onto_gpui_component_colors_and_syntax() {
        let t = theme(
            ThemeKind::Dark,
            &[
                ("bg-island", "#191a1c"),
                ("fg", "#d1d3d9"),
                ("accent", "#3871e1"),
                ("syn-keyword", "#cf8e6d"),
                ("syn-comment", "#7a7e85"),
                ("syn-comment-style", "italic"),
                ("bg-editor", "#1e1f22"),
                ("space-2", "8px"),
            ],
        );
        let config = serde_json::to_value(theme_config(&t, None)).unwrap();
        assert_eq!(config["mode"], "dark");
        assert_eq!(config["colors"]["background"], "#191a1c");
        assert_eq!(config["colors"]["primary.background"], "#3871e1");
        let highlight = &config["highlight"];
        // Serialized under whichever spelling this gpui-component build uses.
        assert!(!highlight["editor.background"].is_null() || !highlight["editor_background"].is_null());
        assert!(!highlight["syntax"]["keyword"]["color"].is_null());
        assert_eq!(highlight["syntax"]["comment"]["font_style"], "italic");

        let custom = serde_json::to_value(theme_config(&t, Some("#24a394"))).unwrap();
        assert_eq!(custom["colors"]["primary.background"], "#24a394");
        let default_accent = serde_json::to_value(theme_config(&t, Some("#3871E1"))).unwrap();
        assert_eq!(default_accent["colors"]["primary.background"], "#3871e1", "the default accent keeps the theme's");
    }
}

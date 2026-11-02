//! Icons by name, as the React frontend names them (`src/ui/icons.tsx`): the
//! JetBrains new-UI SVGs upstream vendors, resolved through the tables exported
//! to `data/icons.json` by `client/scripts/export-presets.ts`.
//!
//! Like React: monochrome icons (only the standard greys `#6C707E` light /
//! `#CED0D6` dark) are drawn in the caller's colour; coloured ones (file types,
//! run, status) keep their palette in their light or `_dark` variant; from 18px
//! the `@20x20` variant is used when there is one (tool window stripes).

use crate::assets::{ICONS, UpstreamIcons};
use gpui::{App, Hsla, IntoElement, Pixels, RenderOnce, SharedString, Styled, Window, img, px, svg};
use regex::Regex;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Tables {
    icons: HashMap<String, String>,
    file_names: HashMap<String, String>,
    name_patterns: Vec<(String, String)>,
    extensions: HashMap<String, String>,
    code_extensions: HashSet<String>,
}

struct IconSet {
    tables: Tables,
    patterns: Vec<(Regex, String)>,
}

static SET: LazyLock<IconSet> = LazyLock::new(|| {
    let tables: Tables = serde_json::from_str(include_str!("../data/icons.json")).expect("data/icons.json");
    let patterns =
        tables.name_patterns.iter().filter_map(|(re, icon)| Regex::new(re).ok().map(|re| (re, icon.clone()))).collect();
    IconSet { tables, patterns }
});

/// The icon for a file name (React's `fileIconName`).
pub fn file_icon_name(file_name: &str) -> &'static str {
    let set = &*SET;
    let lower = file_name.to_lowercase();
    if let Some(icon) = set
        .tables
        .file_names
        .get(&lower)
        .or_else(|| set.patterns.iter().find(|(re, _)| re.is_match(&lower)).map(|(_, i)| i))
    {
        return icon;
    }
    let Some(dot) = lower.rfind('.').filter(|d| *d > 0) else { return "file-text" };
    let ext = &lower[dot + 1..];
    let ext2 = lower[..dot].rfind('.').filter(|p| *p > 0).map(|p| &lower[p + 1..]);
    if let Some(icon) = ext2.and_then(|e| set.tables.extensions.get(e)).or_else(|| set.tables.extensions.get(ext)) {
        return icon;
    }
    if set.tables.code_extensions.contains(ext) { "file-code" } else { "file-unknown" }
}

const MONO_LIGHT: &str = "#6c707e";
const MONO_DARK: &str = "#ced0d6";

/// Whether every colour in `svg` is `mono` (and there is at least one).
fn only_colour(svg: &str, mono: &str) -> bool {
    let colours: HashSet<String> = Regex::new(r"(?i)#[0-9a-f]{3,8}\b")
        .expect("colour pattern")
        .find_iter(svg)
        .map(|m| m.as_str().to_ascii_lowercase())
        .collect();
    !colours.is_empty() && colours.iter().all(|c| c == mono)
}

#[derive(Clone)]
struct Resolved {
    path: SharedString,
    mono: bool,
}

fn exists(path: &str) -> bool {
    UpstreamIcons::get(path).is_some()
}

fn text(path: &str) -> String {
    UpstreamIcons::get(path).map(|f| String::from_utf8_lossy(&f.data).into_owned()).unwrap_or_default()
}

/// The asset to draw for `name` at `large` (>= 18px) size in a `dark` theme.
fn resolve(name: &str, large: bool, dark: bool) -> Option<Resolved> {
    type Cache = HashMap<(String, bool, bool), Option<Resolved>>;
    static CACHE: LazyLock<Mutex<Cache>> = LazyLock::new(Default::default);
    let key = (name.to_owned(), large, dark);
    if let Some(hit) = CACHE.lock().ok()?.get(&key) {
        return hit.clone();
    }
    let resolved = SET.tables.icons.get(name).and_then(|rel| {
        let base = if rel.starts_with("openidea/") { rel.clone() } else { format!("expui/{rel}") };
        let base = if large && exists(&format!("{base}@20x20.svg")) { format!("{base}@20x20") } else { base };
        let light = format!("{base}.svg");
        if !exists(&light) {
            return None;
        }
        let dark_path = format!("{base}_dark.svg");
        let has_dark = exists(&dark_path);
        let mono = only_colour(&text(&light), MONO_LIGHT) && (!has_dark || only_colour(&text(&dark_path), MONO_DARK));
        let path = if !mono && dark && has_dark { dark_path } else { light };
        Some(Resolved { path: format!("{ICONS}{path}").into(), mono })
    });
    if let Ok(mut cache) = CACHE.lock() {
        cache.insert(key, resolved.clone());
    }
    resolved
}

/// An icon by React name (`toolwindow-project`, `folder`, `file-rust`…).
#[derive(IntoElement)]
pub struct Icon {
    name: SharedString,
    size: Pixels,
    color: Option<Hsla>,
}

impl Icon {
    pub fn new(name: impl Into<SharedString>) -> Self {
        Self { name: name.into(), size: px(16.), color: None }
    }

    pub fn size(mut self, size: Pixels) -> Self {
        self.size = size;
        self
    }

    /// The colour of a monochrome icon (coloured icons keep their own).
    pub fn color(mut self, color: Hsla) -> Self {
        self.color = Some(color);
        self
    }
}

impl RenderOnce for Icon {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let dark = crate::theme::is_dark(cx);
        let color = self.color.unwrap_or_else(|| crate::theme::token(cx, "fg-icon"));
        match resolve(&self.name, self.size >= px(18.), dark) {
            Some(Resolved { path, mono: true }) => {
                svg().path(path).size(self.size).flex_none().text_color(color).into_any_element()
            }
            Some(Resolved { path, mono: false }) => img(path).size(self.size).flex_none().into_any_element(),
            None => gpui::div().size(self.size).flex_none().into_any_element(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_icons_follow_react_rules() {
        assert_eq!(file_icon_name("main.rs"), "file-rust");
        assert_eq!(file_icon_name("Cargo.toml"), "file-cargo");
        assert_eq!(file_icon_name("Dockerfile.dev"), "file-docker");
        assert_eq!(file_icon_name("LICENSE"), "file-license");
        assert_eq!(file_icon_name("noext"), "file-text");
        assert_eq!(file_icon_name("x.zzz"), "file-unknown");
    }

    #[test]
    fn icons_resolve_to_upstream_svgs() {
        let folder = resolve("folder", false, true).expect("folder icon");
        assert!(folder.path.ends_with(".svg"));
        let project = resolve("toolwindow-project", true, true).expect("project icon");
        assert!(project.mono, "tool window icons are monochrome");
        assert!(project.path.contains("@20x20"), "{}", project.path);
        let rust = resolve("file-rust", false, true).expect("rust icon");
        assert!(!rust.mono, "file type icons keep their colours");
        assert!(resolve("no-such-icon", false, true).is_none());
    }
}

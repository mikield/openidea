//! The client's assets: upstream's icon SVGs (`src/ui/expui`, `src/ui/openidea`,
//! read-only, under [`ICONS`]) in front of gpui-component's own.

use gpui::{AssetSource, SharedString};
use std::borrow::Cow;

/// Embedded in release builds; read from the repository in debug builds.
#[derive(rust_embed::RustEmbed)]
#[folder = "../../../src/ui/"]
#[include = "expui/**/*.svg"]
#[include = "openidea/**/*.svg"]
pub(crate) struct UpstreamIcons;

/// The asset path prefix of upstream's icons: `oi-icons/expui/nodes/folder.svg`.
pub const ICONS: &str = "oi-icons/";

/// Install with `gpui_platform::application().with_assets(client_core::Assets)`.
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
        match path.strip_prefix(ICONS) {
            Some(icon) => Ok(UpstreamIcons::get(icon).map(|file| file.data)),
            None => gpui_kit_assets::Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> anyhow::Result<Vec<SharedString>> {
        match path.strip_prefix(ICONS) {
            Some(dir) => {
                Ok(UpstreamIcons::iter().filter(|p| p.starts_with(dir)).map(|p| format!("{ICONS}{p}").into()).collect())
            }
            None => gpui_kit_assets::Assets.list(path),
        }
    }
}

/// React's fonts (OFL, `fonts/*-LICENSE.txt`, `fonts/*-OFL.txt`): Inter for the UI and JetBrains
/// Mono for the editor, so the client looks the same without them installed.
const FONTS: [&[u8]; 8] = [
    include_bytes!("../fonts/Inter-Regular.ttf"),
    include_bytes!("../fonts/Inter-Medium.ttf"),
    include_bytes!("../fonts/Inter-SemiBold.ttf"),
    include_bytes!("../fonts/Inter-Bold.ttf"),
    include_bytes!("../fonts/JetBrainsMono-Regular.ttf"),
    include_bytes!("../fonts/JetBrainsMono-Italic.ttf"),
    include_bytes!("../fonts/JetBrainsMono-Bold.ttf"),
    include_bytes!("../fonts/JetBrainsMono-BoldItalic.ttf"),
];

/// Registers the bundled fonts; call once at start, before the first window.
pub fn register_fonts(cx: &gpui::App) {
    if let Err(e) = cx.text_system().add_fonts(FONTS.iter().map(|f| Cow::Borrowed(*f)).collect()) {
        tracing::warn!("bundled fonts: {e}");
    }
}

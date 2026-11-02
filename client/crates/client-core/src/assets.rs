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

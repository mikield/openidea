//! Where the workbench layout is kept: client-side, one file per project root.
//!
//! The layout is the client's own state (React keeps it in `localStorage` per
//! project root), so it stays on the client even when the backend is remote.
//! Files live under [`client_home`]`/layouts`.

use gpui_component::dock::DockAreaState;
use std::path::{Path, PathBuf};

/// The client's own config and state: `$OPENIDEA_CLIENT_HOME`, else the
/// platform config dir's `openidea-gpui`.
pub fn client_home() -> PathBuf {
    std::env::var_os("OPENIDEA_CLIENT_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs::config_dir().unwrap_or_else(std::env::temp_dir).join("openidea-gpui"))
}

/// The saved layout of one project.
#[derive(Debug, Clone)]
pub struct LayoutStore {
    path: PathBuf,
}

impl LayoutStore {
    pub fn for_project(root: &Path) -> Self {
        Self::in_dir(&client_home().join("layouts"), root)
    }

    pub fn in_dir(dir: &Path, root: &Path) -> Self {
        Self { path: dir.join(format!("{:016x}.json", fnv1a(root.to_string_lossy().as_bytes()))) }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The saved layout; `None` when there is none or it does not parse.
    pub fn load(&self) -> Option<DockAreaState> {
        let text = std::fs::read_to_string(&self.path).ok()?;
        serde_json::from_str(&text).inspect_err(|e| tracing::warn!("layout {}: {e}", self.path.display())).ok()
    }

    /// Writes atomically: a crash mid-write keeps the previous layout.
    pub fn save(&self, state: &DockAreaState) -> std::io::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(state)?)?;
        std::fs::rename(&tmp, &self.path)
    }
}

/// FNV-1a: stable across builds and platforms, unlike `std`'s hasher.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_component::dock::{DockPlacement, DockState, PanelInfo, PanelState};

    #[test]
    fn file_names_are_stable_per_root() {
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
        let dir = Path::new("layouts");
        let a = LayoutStore::in_dir(dir, Path::new("/work/a"));
        assert_eq!(a.path(), LayoutStore::in_dir(dir, Path::new("/work/a")).path());
        assert_ne!(a.path(), LayoutStore::in_dir(dir, Path::new("/work/b")).path());
    }

    #[test]
    fn layout_round_trips_and_garbage_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let store = LayoutStore::in_dir(dir.path(), Path::new("/work/a"));
        assert!(store.load().is_none());

        let mut tabs = PanelState::new("TabPanel");
        tabs.info = PanelInfo::tabs(0);
        tabs.add_child(PanelState::new("project"));
        let state = DockAreaState {
            version: Some(1),
            center: PanelState::new(crate::EDITOR_AREA),
            left_dock: Some(DockState::new(tabs, DockPlacement::Left, gpui::px(260.), true)),
            right_dock: None,
            bottom_dock: None,
        };
        store.save(&state).unwrap();
        assert_eq!(store.load(), Some(state));

        std::fs::write(store.path(), "{ not json").unwrap();
        assert!(store.load().is_none());
    }
}

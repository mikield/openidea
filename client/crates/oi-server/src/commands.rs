//! Commands `oi-server` adds to upstream's [`Dispatcher::standard`].
//!
//! Names start with `oi_` so they never collide with upstream commands. A
//! client finds out it talks to `oi-server` (and not a plain `openidea-server`,
//! say on a remote host) by [`SERVER_INFO`] being in `commands`.
//!
//! Every `invoke` passes upstream's trust gate (`Host::authorize_command`): only
//! commands with an allowlisted prefix (`fs_`, `doc_`, `settings_`, …) run
//! without a trusted project open in the window, and `oi_` is not one of them.

use ide_core::IdeResult;
use ide_host::rpc::Dispatcher;
use serde::Serialize;

/// `oi_server_info`: which server and version answers.
pub const SERVER_INFO: &str = "oi_server_info";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OiServerInfo {
    pub name: &'static str,
    pub version: &'static str,
}

pub fn register(d: &mut Dispatcher) {
    d.add(SERVER_INFO, |_| async move { server_info() });
}

fn server_info() -> IdeResult<OiServerInfo> {
    Ok(OiServerInfo { name: "oi-server", version: env!("CARGO_PKG_VERSION") })
}

//! Platform backends behind small seams: the `Desktop` trait, the local tool table,
//! start-up warm-up, and owner-only files for secrets. `cfg(target_os)` picks one per
//! build, so no platform's code is compiled into another's binary.

#[cfg(target_os = "linux")]
pub mod desktop;
#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(windows)]
pub mod windows;

#[cfg(target_os = "linux")]
use linux as current;
#[cfg(windows)]
use windows as current;

pub(crate) use current::private_fs;
pub(crate) use current::{
    DEFAULT_BIND, LOCAL_TOOLS, SERVER_INSTRUCTIONS, SERVER_NAME, SERVER_VERSION,
};

/// The desktop backend for this node (Linux tools are written against this trait).
#[cfg(target_os = "linux")]
pub fn desktop() -> &'static dyn desktop::Desktop {
    current::desktop()
}

/// Platform start-up before the HTTP server begins accepting.
pub fn warm_up() {
    current::warm_up();
}

/// Start the Windows system tray (only called under `#[cfg(windows)]` from main).
#[cfg(windows)]
pub fn windows_start_tray() {
    let opts = crate::cli::options();
    let bind = std::env::var("BIND").unwrap_or_else(|_| DEFAULT_BIND.to_string());
    let url = opts
        .public_url
        .clone()
        .unwrap_or_else(|| format!("http://{bind}{}", opts.mcp_path));
    let token = std::env::var("MCP_TOKEN")
        .ok()
        .filter(|t| !t.is_empty())
        .or_else(|| {
            std::env::var("MCP_TOKEN_FILE")
                .ok()
                .and_then(|p| std::fs::read_to_string(p).ok())
                .map(|s| s.trim().to_string())
                .filter(|t| !t.is_empty())
        });
    let log = windows::private_fs::state_dir()
        .map(|d| {
            d.join("logs")
                .join("agent.log")
                .to_string_lossy()
                .into_owned()
        })
        .unwrap_or_default();
    windows::tray::spawn(url, token, log);
}

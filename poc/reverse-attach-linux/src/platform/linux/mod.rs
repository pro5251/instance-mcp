//! Linux backends. Only wlroots today; a portal backend (GNOME/KDE) would sit beside it.

pub mod private_fs;
pub mod seat;
pub(crate) mod tools;
pub mod wlroots;

use super::desktop::Desktop;

pub(crate) use tools::LOCAL_TOOLS;

/// `initialize` serverInfo, pinned by tests/golden/initialize.json.
pub(crate) const SERVER_NAME: &str = "instance-mcp-rpi";
pub(crate) const SERVER_VERSION: &str = "0.3.0";
/// Linux serves no initialize instructions today.
pub(crate) const SERVER_INSTRUCTIONS: Option<&str> = None;
pub(crate) const DEFAULT_BIND: &str = "127.0.0.1:8790";

static WLROOTS: wlroots::Wlroots = wlroots::Wlroots;

/// The desktop backend for this node. wlroots only today; selection by probing (portal vs
/// wlroots) belongs here when a second backend exists.
pub fn desktop() -> &'static dyn Desktop {
    &WLROOTS
}

/// Input devices first, so clients see pointer/keyboard capabilities before any tool
/// call (see seat.rs). Non-fatal: a node without a seat still serves bash.
pub fn warm_up() {
    seat::warm_up(wlroots::seat_command);
}

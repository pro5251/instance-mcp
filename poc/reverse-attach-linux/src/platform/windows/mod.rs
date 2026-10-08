//! Windows backend (POC). Names are the POC names of spec §13.2 and are pending the
//! author's confirmation; tool names are the functional contract and do not change.

mod capture;
mod desk;
mod exec;
mod input;
mod jobs;
pub(crate) mod private_fs;
mod proc;
mod sysinfo;
mod tools;
pub(crate) mod tray;

pub(crate) use tools::LOCAL_TOOLS;

/// POC names (spec §13.2), all pending the author's confirmation.
pub(crate) const SERVER_NAME: &str = "oab-imcp-winpoc";
pub(crate) const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");
pub(crate) const DEFAULT_BIND: &str = "127.0.0.1:8796";

/// Same role as the macOS daemon's initialize instructions: how to work this
/// computer, in its own terms.
pub(crate) const SERVER_INSTRUCTIONS: Option<&str> = Some(
    "You are operating a real Windows computer through its logged-in desktop session; a human \
     may be watching the screen. Work in a see→act→see loop: `screenshot`, decide, act, then \
     `screenshot` again to confirm — never assume an action landed. Act with `mouse` and \
     `key`; a successful result means the input was sent, not that it landed, because \
     Windows silently drops input aimed at a window running as administrator.\n\
     Coordinates are physical pixels relative to the top-left of the chosen `display` (0 = the \
     primary display). At `scale: 1` an image pixel (x,y) is exactly that coordinate; at the \
     default scale 0.5 divide by 0.5. To read small text pass `region: {x,y,width,height}` with \
     `scale: 1` or more; the crop's pixel (px,py) is (region.x + px/scale, region.y + py/scale).\n\
     When the screen is locked or a UAC prompt is up, the tools fail instead of acting. \
     Call `sys_info` when unsure which displays exist or what is available.",
);

/// Per-monitor v2 DPI awareness before anything measures the screen: without it a
/// scaled monitor is virtualised (a 125% 1920×1200 display reports 1536×960) and
/// screenshot pixels stop matching input coordinates (spec §4.2, spike 1).
pub fn warm_up() {
    use windows_sys::Win32::UI::HiDpi::{
        SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    };
    // SAFETY: plain Win32 call with a documented constant; failure means the process
    // already has an awareness (e.g. from a manifest), which is logged.
    let ok = unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    if ok == 0 {
        eprintln!("dpi: could not set per-monitor v2 awareness (already set by a manifest?)");
    }
}

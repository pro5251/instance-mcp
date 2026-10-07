//! Platform backends behind small seams: the `Desktop` trait, the local tool table,
//! start-up warm-up, and owner-only files for secrets. `cfg(target_os)` picks one per
//! build, so no platform's code is compiled into another's binary.

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
pub(crate) use current::LOCAL_TOOLS;

use desktop::Desktop;

/// The desktop backend for this node.
pub fn desktop() -> &'static dyn Desktop {
    current::desktop()
}

/// Platform start-up before the HTTP server begins accepting.
pub fn warm_up() {
    current::warm_up();
}

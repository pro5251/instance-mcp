//! Windows backend (POC, in progress). This skeleton only makes the crate build for
//! Windows: it serves no local tools yet, and grant persistence stays off until
//! owner-only files are implemented with a protected DACL (spec §6.2), rather than
//! writing attach secrets with whatever ACL the directory happens to inherit.

pub(crate) mod private_fs;

use super::desktop::{Button, Capture, Combo, Desktop, DesktopError};
use crate::tools::LocalTool;

pub(crate) static LOCAL_TOOLS: &[LocalTool] = &[];

struct Unavailable;

const NOT_YET: &str = "this Windows build has no desktop backend yet";

impl Desktop for Unavailable {
    fn display_ok(&self) -> bool {
        false
    }
    fn capture(&self, _: f64, _: &str, _: i64) -> Result<Capture, DesktopError> {
        Err(NOT_YET.to_string())
    }
    fn pointer_goto(&self, _: f64, _: f64) -> Result<(), DesktopError> {
        Err(NOT_YET.to_string())
    }
    fn pointer_move_rel(&self, _: f64, _: f64) -> Result<(), DesktopError> {
        Err(NOT_YET.to_string())
    }
    fn pointer_click(&self, _: Button) -> Result<(), DesktopError> {
        Err(NOT_YET.to_string())
    }
    fn pointer_press(&self, _: Button) -> Result<(), DesktopError> {
        Err(NOT_YET.to_string())
    }
    fn pointer_release(&self, _: Button) -> Result<(), DesktopError> {
        Err(NOT_YET.to_string())
    }
    fn pointer_scroll(&self, _: f64, _: f64) -> Result<(), DesktopError> {
        Err(NOT_YET.to_string())
    }
    fn key_type(&self, _: &str) -> Result<(), DesktopError> {
        Err(NOT_YET.to_string())
    }
    fn key_press(&self, _: &Combo) -> Result<(), DesktopError> {
        Err(NOT_YET.to_string())
    }
}

static UNAVAILABLE: Unavailable = Unavailable;

pub fn desktop() -> &'static dyn Desktop {
    &UNAVAILABLE
}

pub fn warm_up() {}

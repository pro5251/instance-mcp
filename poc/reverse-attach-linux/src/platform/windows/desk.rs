//! Displays and the input desktop, shared by screenshot, sys_info and input.

use windows_sys::Win32::Foundation::{BOOL, LPARAM, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFOEXW,
};
use windows_sys::Win32::System::StationsAndDesktops::{
    CloseDesktop, GetUserObjectInformationW, OpenInputDesktop, DESKTOP_READOBJECTS, UOI_NAME,
};
use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows_sys::Win32::UI::WindowsAndMessaging::MONITORINFOF_PRIMARY;

/// One monitor in physical virtual-desktop pixels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Display {
    pub(crate) device: String,
    pub(crate) left: i32,
    pub(crate) top: i32,
    pub(crate) width: i32,
    pub(crate) height: i32,
    pub(crate) dpi: u32,
    pub(crate) primary: bool,
}

/// Monitors with the primary first, the rest in enumeration order: index 0 is
/// predictable, as on macOS.
pub(crate) fn displays() -> Vec<Display> {
    unsafe extern "system" fn collect(m: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
        // SAFETY: `data` is the `&mut Vec<Display>` passed below, alive for the call.
        let out = unsafe { &mut *(data as *mut Vec<Display>) };
        let mut info: MONITORINFOEXW = unsafe { std::mem::zeroed() };
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        // SAFETY: cbSize set; the struct outlives the call.
        if unsafe { GetMonitorInfoW(m, &mut info as *mut _ as *mut _) } == 0 {
            return 1;
        }
        let (mut dx, mut dy) = (96u32, 96u32);
        // SAFETY: out-pointers to locals.
        unsafe { GetDpiForMonitor(m, MDT_EFFECTIVE_DPI, &mut dx, &mut dy) };
        let r = info.monitorInfo.rcMonitor;
        let name_len = info
            .szDevice
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(info.szDevice.len());
        out.push(Display {
            device: String::from_utf16_lossy(&info.szDevice[..name_len]),
            left: r.left,
            top: r.top,
            width: r.right - r.left,
            height: r.bottom - r.top,
            dpi: dx,
            primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
        });
        1
    }
    let mut list: Vec<Display> = Vec::new();
    // SAFETY: the callback only touches `list` through the LPARAM during this call.
    unsafe {
        EnumDisplayMonitors(
            std::ptr::null_mut(),
            std::ptr::null(),
            Some(collect),
            &mut list as *mut Vec<Display> as LPARAM,
        )
    };
    list.sort_by_key(|d| !d.primary);
    list
}

/// Name of the desktop that currently receives input: `Default` when a user can
/// work, `Winlogon` (lock screen, Ctrl+Alt+Del) or similar otherwise. `None` when
/// it cannot be opened at all, which also means "not usable".
pub(crate) fn input_desktop() -> Option<String> {
    // SAFETY: handle is checked and closed; the buffer outlives the call.
    unsafe {
        let d = OpenInputDesktop(0, 0, DESKTOP_READOBJECTS);
        if d.is_null() {
            return None;
        }
        let mut buf = [0u16; 128];
        let mut needed = 0u32;
        let ok = GetUserObjectInformationW(
            d,
            UOI_NAME,
            buf.as_mut_ptr().cast(),
            std::mem::size_of_val(&buf) as u32,
            &mut needed,
        );
        CloseDesktop(d);
        if ok == 0 {
            return None;
        }
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..len]))
    }
}

/// Fail before acting on a desktop the user cannot see or use (spec §4.3, §8 item 10).
pub(crate) fn require_usable_desktop() -> Result<(), String> {
    match input_desktop() {
        Some(name) if name.eq_ignore_ascii_case("Default") => Ok(()),
        Some(name) => Err(format!(
            "the input desktop is '{name}' (screen locked, Ctrl+Alt+Del or a UAC prompt); \
             nothing can be seen or done until the user returns to the normal desktop"
        )),
        None => Err(
            "cannot open the input desktop (locked screen, secure desktop or no interactive \
             session); nothing can be seen or done"
                .to_string(),
        ),
    }
}

pub(crate) fn display(index: i64) -> Result<Display, String> {
    let list = displays();
    usize::try_from(index)
        .ok()
        .and_then(|i| list.get(i).cloned())
        .ok_or_else(|| {
            format!(
                "display {index} out of range; {} display(s) available",
                list.len()
            )
        })
}

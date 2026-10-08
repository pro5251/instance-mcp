//! System tray icon for the Windows node (`--menu-bar`, the same flag as the macOS menu
//! bar). Mirrors the macOS StatusItem menu: the public URL and bearer token (copied to
//! the clipboard, token masked in the tooltip), an environment check, open-log, restart
//! and quit. Runs its own message loop on a dedicated thread so HTTP keeps serving.
//!
//! Menu interaction is verified manually (ticket 15); start-up + "HTTP still serves"
//! is covered by the Windows smoke.

use std::sync::OnceLock;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DispatchMessageW,
    GetCursorPos, GetMessageW, LoadIconW, MessageBoxW, PostQuitMessage, RegisterClassW,
    SetForegroundWindow, TrackPopupMenu, TranslateMessage, HMENU, IDI_APPLICATION, MB_OK,
    MF_SEPARATOR, MF_STRING, MSG, TPM_RIGHTBUTTON, WM_APP, WM_COMMAND, WM_DESTROY, WNDCLASSW,
    WS_OVERLAPPED,
};

const WM_TRAY: u32 = WM_APP + 1;
const ID_URL: usize = 1;
const ID_TOKEN: usize = 2;
const ID_ENVCHECK: usize = 3;
const ID_LOG: usize = 4;
const ID_RESTART: usize = 5;
const ID_QUIT: usize = 6;

struct TrayState {
    url: String,
    token: Option<String>,
    log_path: String,
}

static STATE: OnceLock<TrayState> = OnceLock::new();

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Mask a token for display: first 4 and last 4 chars, the middle as dots.
fn mask(token: &str) -> String {
    let n = token.chars().count();
    if n <= 8 {
        return "•".repeat(n);
    }
    let first: String = token.chars().take(4).collect();
    let last: String = token.chars().skip(n - 4).collect();
    format!("{first}…{last}")
}

/// Put `text` on the clipboard as Unicode (best effort).
fn set_clipboard(text: &str) {
    let utf16: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes = utf16.len() * 2;
    // SAFETY: standard clipboard sequence; the global is owned by the clipboard after
    // SetClipboardData succeeds, so we do not free it ourselves.
    unsafe {
        if OpenClipboard(std::ptr::null_mut()) == 0 {
            return;
        }
        EmptyClipboard();
        let h = GlobalAlloc(GMEM_MOVEABLE, bytes);
        if !h.is_null() {
            let p = GlobalLock(h) as *mut u16;
            if !p.is_null() {
                std::ptr::copy_nonoverlapping(utf16.as_ptr(), p, utf16.len());
                GlobalUnlock(h);
                // 13 = CF_UNICODETEXT
                SetClipboardData(13, h as _);
            }
        }
        CloseClipboard();
    }
}

fn message_box(title: &str, body: &str) {
    let t = wide(title);
    let b = wide(body);
    // SAFETY: NUL-terminated strings; a modal box owned by no window.
    unsafe { MessageBoxW(std::ptr::null_mut(), b.as_ptr(), t.as_ptr(), MB_OK) };
}

/// The environment-check report the menu shows (and the smoke can print): what will and
/// will not work right now. Degrades gracefully — each line is best-effort.
pub(crate) fn environment_report() -> Vec<String> {
    let mut out = Vec::new();
    let desktop = super::desk::input_desktop();
    out.push(format!(
        "input desktop: {}",
        desktop
            .as_deref()
            .unwrap_or("unavailable (locked / no session)")
    ));
    let displays = super::desk::displays();
    out.push(format!(
        "displays: {} (DPI-aware per-monitor v2)",
        displays.len()
    ));
    out.push(format!(
        "Tailscale: {}",
        if which("tailscale.exe") {
            "found on PATH"
        } else {
            "not found (install for remote access)"
        }
    ));
    out.push(format!(
        "Node.js (for Playwright): {}",
        if which("node.exe") {
            "found on PATH"
        } else {
            "not found (browser_* unavailable)"
        }
    ));
    out
}

fn which(exe: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|d| d.join(exe).is_file()))
        .unwrap_or(false)
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_TRAY => {
            // Any mouse message on the icon pops the menu (lparam low word is the event).
            let ev = (lparam & 0xFFFF) as u32;
            // WM_RBUTTONUP = 0x0205, WM_LBUTTONUP = 0x0202
            if ev == 0x0205 || ev == 0x0202 {
                // SAFETY: valid window handle.
                unsafe { show_menu(hwnd) };
            }
            0
        }
        WM_COMMAND => {
            // SAFETY: id is a menu id we appended.
            unsafe { on_command(wparam & 0xFFFF) };
            0
        }
        WM_DESTROY => {
            // SAFETY: standard teardown.
            unsafe { PostQuitMessage(0) };
            0
        }
        // SAFETY: default handling for everything else.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

unsafe fn show_menu(hwnd: HWND) {
    let menu: HMENU = CreatePopupMenu();
    if menu.is_null() {
        return;
    }
    let st = STATE.get();
    let url = st.map(|s| s.url.clone()).unwrap_or_default();
    AppendMenuW(
        menu,
        MF_STRING,
        ID_URL,
        wide(&format!("Copy URL  ({url})")).as_ptr(),
    );
    if let Some(tok) = st.and_then(|s| s.token.as_deref()) {
        AppendMenuW(
            menu,
            MF_STRING,
            ID_TOKEN,
            wide(&format!("Copy token  ({})", mask(tok))).as_ptr(),
        );
    }
    AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
    AppendMenuW(
        menu,
        MF_STRING,
        ID_ENVCHECK,
        wide("Environment check…").as_ptr(),
    );
    AppendMenuW(menu, MF_STRING, ID_LOG, wide("Open log").as_ptr());
    AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
    AppendMenuW(menu, MF_STRING, ID_RESTART, wide("Restart agent").as_ptr());
    AppendMenuW(menu, MF_STRING, ID_QUIT, wide("Quit agent").as_ptr());

    let mut p = POINT { x: 0, y: 0 };
    GetCursorPos(&mut p);
    // The menu needs the window foreground or it will not dismiss on click-away.
    SetForegroundWindow(hwnd);
    TrackPopupMenu(menu, TPM_RIGHTBUTTON, p.x, p.y, 0, hwnd, std::ptr::null());
    DestroyMenu(menu);
}

unsafe fn on_command(id: usize) {
    let st = STATE.get();
    match id {
        ID_URL => {
            if let Some(s) = st {
                set_clipboard(&s.url);
            }
        }
        ID_TOKEN => {
            if let Some(tok) = st.and_then(|s| s.token.as_deref()) {
                set_clipboard(tok);
            }
        }
        ID_ENVCHECK => {
            message_box("instance-mcp environment", &environment_report().join("\n"));
        }
        ID_LOG => {
            if let Some(s) = st {
                open_path(&s.log_path);
            }
        }
        ID_RESTART => restart(),
        ID_QUIT => std::process::exit(0),
        _ => {}
    }
}

fn open_path(path: &str) {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    let verb = wide("open");
    let file = wide(path);
    // SAFETY: NUL-terminated strings; a fire-and-forget shell open.
    unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1, // SW_SHOWNORMAL
        )
    };
}

/// Re-exec this binary with the same arguments, then exit — the Scheduled Task would
/// also restart it, but this is immediate.
fn restart() {
    let exe = std::env::current_exe().ok();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(exe) = exe {
        let _ = std::process::Command::new(exe).args(&args).spawn();
    }
    std::process::exit(0);
}

/// Start the tray on a dedicated thread. `url` is the public URL; `token` the bearer
/// token to offer for copy; `log_path` the agent log.
pub(crate) fn spawn(url: String, token: Option<String>, log_path: String) {
    let _ = STATE.set(TrayState {
        url,
        token,
        log_path,
    });
    std::thread::spawn(|| unsafe { run_loop() });
}

unsafe fn run_loop() {
    let class_name = wide("oab_imcp_tray");
    let hinst = GetModuleHandleW(std::ptr::null());
    let mut wc: WNDCLASSW = std::mem::zeroed();
    wc.lpfnWndProc = Some(wndproc);
    wc.hInstance = hinst;
    wc.lpszClassName = class_name.as_ptr();
    RegisterClassW(&wc);

    let hwnd = CreateWindowExW(
        0,
        class_name.as_ptr(),
        wide("oab-instance-mcp").as_ptr(),
        WS_OVERLAPPED,
        0,
        0,
        0,
        0,
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        hinst,
        std::ptr::null(),
    );
    if hwnd.is_null() {
        eprintln!("tray: could not create its window; the node runs without a tray icon");
        return;
    }

    let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
    nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = 1;
    nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
    nid.uCallbackMessage = WM_TRAY;
    nid.hIcon = LoadIconW(std::ptr::null_mut(), IDI_APPLICATION);
    let tip = STATE
        .get()
        .map(|s| format!("oab-instance-mcp — {}", s.url))
        .unwrap_or_default();
    for (i, c) in tip.encode_utf16().take(nid.szTip.len() - 1).enumerate() {
        nid.szTip[i] = c;
    }
    Shell_NotifyIconW(NIM_ADD, &nid);

    let mut msg: MSG = std::mem::zeroed();
    while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
    Shell_NotifyIconW(NIM_DELETE, &nid);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_is_masked() {
        assert_eq!(mask("abcd1234efgh5678"), "abcd…5678");
        assert_eq!(mask("short"), "•••••");
    }
}

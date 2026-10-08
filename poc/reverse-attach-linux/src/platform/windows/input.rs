//! `mouse` and `key` on Windows via SendInput (spec §4.3, spike 2).
//!
//! Coordinates are physical pixels relative to the chosen display's top-left, the
//! same space as `screenshot` at scale 1. Arguments follow the Linux schema and also
//! take the macOS ones (`keys`, `modifiers`, `display`, `delay_ms`). Results have the
//! Linux shape (`{"ok": true, "action": …}`).
//!
//! What "ok" means: the events were injected into the input stream. Windows does not
//! report when a higher-integrity window (an app run as administrator) drops them, so
//! callers confirm with a screenshot, as the instructions say.

use std::thread;
use std::time::Duration;

use serde_json::{json, Value};
use windows_sys::Win32::Foundation::LPARAM;
use windows_sys::Win32::Foundation::POINT;
use windows_sys::Win32::UI::Input::Ime::ImmGetDefaultIMEWnd;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyboardLayout, GetKeyboardLayoutList, LoadKeyboardLayoutW, SendInput, UnloadKeyboardLayout,
    HKL, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_KEYUP,
    KEYEVENTF_UNICODE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN,
    MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP,
    MOUSEEVENTF_VIRTUALDESK, MOUSEEVENTF_WHEEL, MOUSEINPUT, VIRTUAL_KEY,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetForegroundWindow, GetSystemMetrics, GetWindowThreadProcessId, PostMessageW,
    SendMessageTimeoutW, SetCursorPos, SMTO_ABORTIFHUNG, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN,
    SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, WHEEL_DELTA, WM_INPUTLANGCHANGEREQUEST,
};

use super::desk;
use crate::tools::tool_result;

const MAX_TYPE_UNITS: usize = 20_000; // same cap as macOS

// ---------------------------------------------------------------------------
// key names
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Combo {
    pub(crate) modifiers: Vec<VIRTUAL_KEY>,
    pub(crate) key: VIRTUAL_KEY,
}

const VK_SHIFT: VIRTUAL_KEY = 0x10;
const VK_CONTROL: VIRTUAL_KEY = 0x11;
const VK_MENU: VIRTUAL_KEY = 0x12;
const VK_LWIN: VIRTUAL_KEY = 0x5B;
const VK_RMENU: VIRTUAL_KEY = 0xA5;

/// Modifier names from xkb (Linux) and macOS. `cmd` is `ctrl` here, so `cmd+c`
/// copies on every platform (instance-mcp#32 item 9); the Windows key is
/// `super` / `logo` / `win`.
fn modifier(name: &str) -> Option<VIRTUAL_KEY> {
    Some(match name.to_ascii_lowercase().as_str() {
        "ctrl" | "control" | "cmd" | "command" | "meta" => VK_CONTROL,
        "shift" => VK_SHIFT,
        "alt" | "opt" | "option" => VK_MENU,
        "super" | "logo" | "win" | "windows" => VK_LWIN,
        "altgr" => VK_RMENU,
        _ => return None,
    })
}

/// Key names from xkb (Linux: `Return`, `BackSpace`, `F4`) and macOS (`return`,
/// `delete` = backspace, `forwarddelete`), letters, digits and US punctuation.
fn key(name: &str) -> Option<VIRTUAL_KEY> {
    // xkb `Delete` is the forward-delete key; macOS `delete` is backspace.
    if name == "Delete" {
        return Some(0x2E);
    }
    let lower = name.to_ascii_lowercase();
    let vk = match lower.as_str() {
        "return" | "enter" | "kp_enter" => 0x0D,
        "tab" => 0x09,
        "space" => 0x20,
        "backspace" | "delete" => 0x08,
        "forwarddelete" => 0x2E,
        "escape" | "esc" => 0x1B,
        "left" => 0x25,
        "up" => 0x26,
        "right" => 0x27,
        "down" => 0x28,
        "home" => 0x24,
        "end" => 0x23,
        "pageup" | "prior" | "page_up" => 0x21,
        "pagedown" | "next" | "page_down" => 0x22,
        "insert" => 0x2D,
        "capslock" | "caps_lock" => 0x14,
        "print" | "printscreen" => 0x2C,
        "menu" => 0x5D,
        "volumeup" => 0xAF,
        "volumedown" => 0xAE,
        "mute" => 0xAD,
        "minus" | "-" => 0xBD,
        "equal" | "=" | "plus" => 0xBB,
        "comma" | "," => 0xBC,
        "period" | "." => 0xBE,
        "slash" | "/" => 0xBF,
        "semicolon" | ";" => 0xBA,
        "apostrophe" | "'" => 0xDE,
        "grave" | "`" => 0xC0,
        "bracketleft" | "[" => 0xDB,
        "bracketright" | "]" => 0xDD,
        "backslash" | "\\" => 0xDC,
        s if s.len() == 1 && s.as_bytes()[0].is_ascii_alphanumeric() => {
            s.as_bytes()[0].to_ascii_uppercase() as VIRTUAL_KEY
        }
        s if s.starts_with('f') && s.len() <= 3 => match s[1..].parse::<u16>() {
            Ok(n @ 1..=24) => 0x70 + n - 1,
            _ => return None,
        },
        _ => return None,
    };
    Some(vk)
}

/// `ctrl+shift+t`, `Return`, `cmd+plus`. The last part is the key, the rest modifiers.
pub(crate) fn parse_combo(combo: &str) -> Result<Combo, String> {
    let parts: Vec<&str> = combo.split('+').map(str::trim).collect();
    let (last, mods) = parts
        .split_last()
        .filter(|(k, _)| !k.is_empty())
        .ok_or_else(|| format!("empty key in '{combo}'"))?;
    let mut modifiers = Vec::new();
    for m in mods {
        modifiers.push(modifier(m).ok_or_else(|| format!("unknown modifier '{m}' in '{combo}'"))?);
    }
    let key = key(last).ok_or_else(|| {
        format!("unknown key '{last}' in '{combo}'; use a letter, digit, punctuation, F1–F24 or a named key such as Return, Tab, Escape, BackSpace, Delete, arrows, Home/End, PageUp/PageDown")
    })?;
    Ok(Combo { modifiers, key })
}

// ---------------------------------------------------------------------------
// SendInput
// ---------------------------------------------------------------------------

fn kbd(vk: VIRTUAL_KEY, scan: u16, flags: u32) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn mouse_input(dx: i32, dy: i32, data: i32, flags: u32) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: data as u32,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

/// Inject all events or report how many went in.
fn send(inputs: &[INPUT]) -> Result<(), String> {
    if inputs.is_empty() {
        return Ok(());
    }
    // SAFETY: a valid slice of INPUT and its exact element size.
    let sent = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        )
    };
    if sent as usize == inputs.len() {
        Ok(())
    } else {
        Err(format!(
            "only {sent} of {} input events were injected (input blocked by another program?)",
            inputs.len()
        ))
    }
}

fn combo_events(c: &Combo) -> Vec<INPUT> {
    let mut v: Vec<INPUT> = c.modifiers.iter().map(|&m| kbd(m, 0, 0)).collect();
    v.push(kbd(c.key, 0, 0));
    v.push(kbd(c.key, 0, KEYEVENTF_KEYUP));
    v.extend(
        c.modifiers
            .iter()
            .rev()
            .map(|&m| kbd(m, 0, KEYEVENTF_KEYUP)),
    );
    v
}

/// Unicode text, one down/up pair per UTF-16 unit (a surrogate pair is two units);
/// newlines become Enter.
fn text_events(text: &str) -> Vec<INPUT> {
    let mut v = Vec::new();
    for unit in text.encode_utf16() {
        if unit == '\n' as u16 {
            v.push(kbd(0x0D, 0, 0));
            v.push(kbd(0x0D, 0, KEYEVENTF_KEYUP));
        } else if unit != '\r' as u16 {
            v.push(kbd(0, unit, KEYEVENTF_UNICODE));
            v.push(kbd(0, unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
        }
    }
    v
}

/// While typing, close the foreground window's IME and restore it afterwards: with a
/// CJK IME open, full-width punctuation sent as Unicode is held in composition and
/// arrives late and out of order (spike 2). Works across processes.
struct ImeClosed {
    ime: windows_sys::Win32::Foundation::HWND,
    was_open: bool,
    /// Conversion mode before typing (TSF IMEs such as zh-TW Bopomofo switch
    /// Chinese/English with it, not with the open status).
    conversion: usize,
    /// Open status read back after closing (0 = closed as asked).
    after: usize,
}

const WM_IME_CONTROL: u32 = 0x0283;
const IMC_GETOPENSTATUS: usize = 0x0005;
const IMC_SETOPENSTATUS: usize = 0x0006;
const IMC_GETCONVERSIONMODE: usize = 0x0001;
const IMC_SETCONVERSIONMODE: usize = 0x0002;
const IME_CMODE_ALPHANUMERIC: isize = 0;

fn ime_message(ime: windows_sys::Win32::Foundation::HWND, cmd: usize, value: isize) -> usize {
    let mut result = 0usize;
    // SAFETY: plain message to a window handle; SMTO_ABORTIFHUNG bounds a hung target.
    unsafe {
        SendMessageTimeoutW(
            ime,
            WM_IME_CONTROL,
            cmd,
            value,
            SMTO_ABORTIFHUNG,
            500,
            &mut result,
        )
    };
    result
}

impl ImeClosed {
    fn new() -> Option<Self> {
        // SAFETY: no arguments / a window handle from the system.
        let ime = unsafe { ImmGetDefaultIMEWnd(GetForegroundWindow()) };
        if ime.is_null() {
            return None;
        }
        // Close unconditionally: right after a window gains focus the open status can
        // read 0 while the IME still captures input (seen in spike 2 and the smoke
        // test). Restore only what was reported, so a closed IME stays closed.
        let was_open = ime_message(ime, IMC_GETOPENSTATUS, 0) != 0;
        let conversion = ime_message(ime, IMC_GETCONVERSIONMODE, 0);
        ime_message(ime, IMC_SETCONVERSIONMODE, IME_CMODE_ALPHANUMERIC);
        ime_message(ime, IMC_SETOPENSTATUS, 0);
        thread::sleep(Duration::from_millis(30));
        let after = ime_message(ime, IMC_GETOPENSTATUS, 0);
        Some(Self {
            ime,
            was_open,
            conversion,
            after,
        })
    }
}

impl Drop for ImeClosed {
    fn drop(&mut self) {
        if self.was_open {
            ime_message(self.ime, IMC_SETOPENSTATUS, 1);
        }
        ime_message(self.ime, IMC_SETCONVERSIONMODE, self.conversion as isize);
    }
}

/// While typing into a window whose keyboard layout is a CJK IME, switch that window
/// to US English and switch back afterwards. Closing the IME is not enough for TSF
/// IMEs: with zh-TW Bopomofo some full-width punctuation (「。」) was still held in
/// composition and arrived out of order (smoke test, Windows 11 build 26300).
struct UsLayout {
    window: windows_sys::Win32::Foundation::HWND,
    thread: u32,
    previous: HKL,
    /// en-US was not installed and was loaded only for this; unloaded afterwards.
    loaded: Option<HKL>,
}

const KLF_NOTELLSHELL: u32 = 0x0080;

fn lang(hkl: HKL) -> u16 {
    (hkl as usize & 0xFFFF) as u16
}

fn wait_for_layout(thread: u32, want: HKL) -> bool {
    for _ in 0..30 {
        // SAFETY: plain query.
        if unsafe { GetKeyboardLayout(thread) } == want {
            return true;
        }
        thread::sleep(Duration::from_millis(10));
    }
    false
}

impl UsLayout {
    fn new() -> Option<Self> {
        // SAFETY: plain queries on the foreground window and its thread.
        let window = unsafe { GetForegroundWindow() };
        let thread = unsafe { GetWindowThreadProcessId(window, std::ptr::null_mut()) };
        let previous = unsafe { GetKeyboardLayout(thread) };
        // zh-TW, zh-CN, zh-HK, zh-SG, zh-MO, ja-JP, ko-KR.
        if !matches!(
            lang(previous),
            0x0404 | 0x0804 | 0x0C04 | 0x1004 | 0x1404 | 0x0411 | 0x0412
        ) {
            return None;
        }
        let mut installed: [HKL; 32] = [std::ptr::null_mut(); 32];
        // SAFETY: the buffer holds 32 entries.
        let n = unsafe { GetKeyboardLayoutList(32, installed.as_mut_ptr()) }.max(0) as usize;
        let (us, loaded) = match installed[..n].iter().find(|&&h| lang(h) == 0x0409) {
            Some(&h) => (h, None),
            None => {
                let id: Vec<u16> = "00000409\0".encode_utf16().collect();
                // SAFETY: NUL-terminated id.
                let h = unsafe { LoadKeyboardLayoutW(id.as_ptr(), KLF_NOTELLSHELL) };
                if h.is_null() {
                    return None;
                }
                (h, Some(h))
            }
        };
        // SAFETY: posting a documented request to a window handle.
        unsafe { PostMessageW(window, WM_INPUTLANGCHANGEREQUEST, 0, us as LPARAM) };
        if !wait_for_layout(thread, us) {
            if let Some(h) = loaded {
                // SAFETY: a layout this guard loaded.
                unsafe { UnloadKeyboardLayout(h) };
            }
            return None;
        }
        // Let the target finish processing the layout change before we type, or the
        // first few characters can still go through the old IME (seen as an occasional
        // flake right after the window gains focus). The change is posted, not sent, so
        // give the target's input thread time to pump it.
        thread::sleep(Duration::from_millis(250));
        Some(Self {
            window,
            thread,
            previous,
            loaded,
        })
    }
}

impl Drop for UsLayout {
    fn drop(&mut self) {
        // SAFETY: as in `new`.
        unsafe {
            PostMessageW(
                self.window,
                WM_INPUTLANGCHANGEREQUEST,
                0,
                self.previous as LPARAM,
            )
        };
        wait_for_layout(self.thread, self.previous);
        if let Some(h) = self.loaded {
            // SAFETY: a layout this guard loaded.
            unsafe { UnloadKeyboardLayout(h) };
        }
    }
}

// ---------------------------------------------------------------------------
// mouse
// ---------------------------------------------------------------------------

fn num(args: &Value, k: &str) -> Option<f64> {
    args.get(k).and_then(Value::as_f64)
}

/// A pointer target: the exact virtual-desktop pixel and its normalised form.
#[derive(Clone, Copy)]
struct Target {
    px: (i32, i32),
    norm: (i32, i32),
}

/// Display-relative pixel → pointer target on the virtual desktop.
fn absolute(d: &desk::Display, x: f64, y: f64) -> Result<Target, String> {
    if x < 0.0 || y < 0.0 || x >= d.width as f64 || y >= d.height as f64 {
        return Err(format!(
            "point ({x}, {y}) is outside display {} ({}×{} px)",
            d.device, d.width, d.height
        ));
    }
    // SAFETY: plain metric queries.
    let (vx, vy, vw, vh) = unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    };
    let px = (d.left + x.round() as i32, d.top + y.round() as i32);
    Ok(Target {
        px,
        norm: normalise(px.0, px.1, (vx, vy, vw, vh)),
    })
}

/// Pixel → 0..=65535 across the virtual desktop, rounded to the pixel centre
/// (exact on all monitors in spike 2).
pub(crate) fn normalise(px: i32, py: i32, (vx, vy, vw, vh): (i32, i32, i32, i32)) -> (i32, i32) {
    let n = |p: i32, o: i32, size: i32| {
        (((p - o) as i64 * 65535 + (size as i64 - 1) / 2) / (size as i64 - 1).max(1)) as i32
    };
    (n(px, vx, vw), n(py, vy, vh))
}

fn move_event(t: Target) -> INPUT {
    mouse_input(
        t.norm.0,
        t.norm.1,
        0,
        MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
    )
}

/// Move as real input, then make the position exact: Windows' de-normalisation can
/// land one pixel off near a display edge (seen on a mixed-DPI desktop).
fn move_to(t: Target) -> Result<(), String> {
    send(&[move_event(t)])?;
    if cursor() != Some(t.px) {
        // SAFETY: plain call; physical pixels (per-monitor-v2 process).
        unsafe { SetCursorPos(t.px.0, t.px.1) };
    }
    Ok(())
}

/// Where the cursor is, in physical virtual-desktop pixels.
fn cursor() -> Option<(i32, i32)> {
    let mut p = POINT { x: 0, y: 0 };
    // SAFETY: out-pointer to a local.
    (unsafe { GetCursorPos(&mut p) } != 0).then_some((p.x, p.y))
}

fn modifiers_arg(args: &Value) -> Result<Vec<VIRTUAL_KEY>, String> {
    let mut out = Vec::new();
    for m in args
        .get("modifiers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let name = m.as_str().unwrap_or_default();
        out.push(modifier(name).ok_or_else(|| format!("unknown modifier '{name}'"))?);
    }
    Ok(out)
}

pub(crate) fn tool_mouse(args: &Value) -> Result<Value, (i64, String)> {
    let bad = |e: String| (-32602, e);
    let action = args
        .get("action")
        .and_then(Value::as_str)
        .ok_or_else(|| bad("missing action".to_string()))?;
    let mods = modifiers_arg(args).map_err(bad)?;
    desk::require_usable_desktop().map_err(|e| (-32000, e))?;
    let d = desk::display(args.get("display").and_then(Value::as_i64).unwrap_or(0)).map_err(bad)?;
    let point = |xk: &str, yk: &str| -> Result<Option<Target>, (i64, String)> {
        match (num(args, xk), num(args, yk)) {
            (Some(x), Some(y)) => absolute(&d, x, y).map(Some).map_err(bad),
            _ => Ok(None),
        }
    };
    let need =
        |p: Option<Target>, what: &str| p.ok_or_else(|| bad(format!("{action} needs {what}")));
    let io = |r: Result<(), String>| r.map_err(|e| (-32000, e));
    let mods_down: Vec<INPUT> = mods.iter().map(|&m| kbd(m, 0, 0)).collect();
    let mods_up: Vec<INPUT> = mods
        .iter()
        .rev()
        .map(|&m| kbd(m, 0, KEYEVENTF_KEYUP))
        .collect();
    match action {
        "move" => io(move_to(need(point("x", "y")?, "x and y")?))?,
        "click" | "double_click" | "right_click" => {
            if let Some(p) = point("x", "y")? {
                io(move_to(p))?;
                thread::sleep(Duration::from_millis(30));
            }
            let (down, up) = if action == "right_click" {
                (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP)
            } else {
                (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP)
            };
            let clicks = if action == "double_click" { 2 } else { 1 };
            let mut ev = mods_down.clone();
            for _ in 0..clicks {
                ev.push(mouse_input(0, 0, 0, down));
                ev.push(mouse_input(0, 0, 0, up));
            }
            ev.extend(mods_up.iter().copied());
            io(send(&ev))?;
        }
        "drag" => {
            let from = need(point("x", "y")?, "x and y")?;
            let to = need(point("to_x", "to_y")?, "to_x and to_y")?;
            io(move_to(from))?;
            thread::sleep(Duration::from_millis(30));
            io(send(&[mouse_input(0, 0, 0, MOUSEEVENTF_LEFTDOWN)]))?;
            // Interpolated like macOS, so apps see a real drag, not a jump.
            let steps = 12;
            let mut result = Ok(());
            for i in 1..=steps {
                let t = i as f64 / steps as f64;
                let lerp = |a: i32, b: i32| a + ((b - a) as f64 * t).round() as i32;
                let step = Target {
                    px: (lerp(from.px.0, to.px.0), lerp(from.px.1, to.px.1)),
                    norm: (lerp(from.norm.0, to.norm.0), lerp(from.norm.1, to.norm.1)),
                };
                let r = if i == steps {
                    move_to(to)
                } else {
                    send(&[move_event(step)])
                };
                if let Err(e) = r {
                    result = Err(e);
                    break;
                }
                thread::sleep(Duration::from_millis(15));
            }
            // Always release, so a failure cannot leave the button held.
            let up = send(&[mouse_input(0, 0, 0, MOUSEEVENTF_LEFTUP)]);
            io(result.and(up))?;
        }
        "scroll" => {
            if let Some(p) = point("x", "y")? {
                io(move_to(p))?;
                thread::sleep(Duration::from_millis(20));
            }
            // Linux semantics: positive dy scrolls down, positive dx right.
            let dy = num(args, "dy").unwrap_or(0.0);
            let dx = num(args, "dx").unwrap_or(0.0);
            if dx == 0.0 && dy == 0.0 {
                return Err(bad("scroll needs dx or dy".to_string()));
            }
            let mut ev = Vec::new();
            if dy != 0.0 {
                ev.push(mouse_input(
                    0,
                    0,
                    -(dy * WHEEL_DELTA as f64).round() as i32,
                    MOUSEEVENTF_WHEEL,
                ));
            }
            if dx != 0.0 {
                ev.push(mouse_input(
                    0,
                    0,
                    (dx * WHEEL_DELTA as f64).round() as i32,
                    MOUSEEVENTF_HWHEEL,
                ));
            }
            io(send(&ev))?;
        }
        other => return Err(bad(format!("unknown mouse action: {other}"))),
    }
    // Where the pointer ended up, read right away (display pixels), so a caller can
    // tell "moved" from "landed" even while someone else uses the mouse.
    let mut out = json!({ "ok": true, "action": action, "display": d.device });
    if let Some((x, y)) = cursor() {
        out["position"] = json!({ "x": x - d.left, "y": y - d.top });
    }
    Ok(tool_result(out))
}

// ---------------------------------------------------------------------------
// key
// ---------------------------------------------------------------------------

pub(crate) fn tool_key(args: &Value) -> Result<Value, (i64, String)> {
    let bad = |e: String| (-32602, e);
    let action = args
        .get("action")
        .and_then(Value::as_str)
        .ok_or_else(|| bad("missing action".to_string()))?;
    let delay = args
        .get("delay_ms")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        .min(2000);
    match action {
        "type" => {
            let text = args
                .get("text")
                .and_then(Value::as_str)
                .filter(|t| !t.is_empty())
                .ok_or_else(|| bad("type needs text".to_string()))?;
            if text.encode_utf16().count() > MAX_TYPE_UNITS {
                return Err(bad(format!(
                    "text too long (max {MAX_TYPE_UNITS} UTF-16 units)"
                )));
            }
            desk::require_usable_desktop().map_err(|e| (-32000, e))?;
            let layout = UsLayout::new();
            let ime = ImeClosed::new();
            let ime_state = match &ime {
                _ if layout.is_some() => "keyboard layout switched to en-US while typing",
                None => "no IME window",
                Some(g) if g.after != 0 => "IME could not be paused",
                Some(g) if g.was_open => "IME paused",
                Some(_) => "IME was closed",
            };
            let r = if delay == 0 {
                send(&text_events(text))
            } else {
                text.chars().try_for_each(|c| {
                    send(&text_events(c.encode_utf8(&mut [0; 4])))?;
                    thread::sleep(Duration::from_millis(delay));
                    Ok(())
                })
            };
            // Let the target drain its queue before the IME is reopened.
            thread::sleep(Duration::from_millis(50));
            drop(ime);
            drop(layout);
            r.map_err(|e| (-32000, e))?;
            Ok(tool_result(json!({
                "ok": true,
                "action": "type",
                "chars": text.chars().count(),
                "ime": ime_state
            })))
        }
        "press" => {
            // Linux sends one `combo`; macOS sends `keys`, run in order.
            let combos: Vec<String> = match (args.get("combo"), args.get("keys")) {
                (Some(Value::String(c)), _) => vec![c.clone()],
                (_, Some(Value::Array(ks))) if !ks.is_empty() => ks
                    .iter()
                    .map(|k| {
                        k.as_str()
                            .map(str::to_string)
                            .ok_or_else(|| bad("keys must be strings".to_string()))
                    })
                    .collect::<Result<_, _>>()?,
                _ => return Err(bad("press needs combo (or keys)".to_string())),
            };
            let parsed: Vec<Combo> = combos
                .iter()
                .map(|c| parse_combo(c).map_err(bad))
                .collect::<Result<_, _>>()?;
            desk::require_usable_desktop().map_err(|e| (-32000, e))?;
            for (i, c) in parsed.iter().enumerate() {
                send(&combo_events(c)).map_err(|e| (-32000, e))?;
                if i + 1 < parsed.len() {
                    thread::sleep(Duration::from_millis(if delay == 0 { 50 } else { delay }));
                }
            }
            let mut out = json!({ "ok": true, "action": "press" });
            if combos.len() == 1 {
                out["combo"] = json!(combos[0]);
            } else {
                out["keys"] = json!(combos);
            }
            Ok(tool_result(out))
        }
        other => Err(bad(format!("unknown key action: {other}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combos_accept_xkb_and_macos_names() {
        let c = parse_combo("ctrl+shift+t").unwrap();
        assert_eq!(
            (c.modifiers, c.key),
            (vec![VK_CONTROL, VK_SHIFT], b'T' as u16)
        );
        assert_eq!(parse_combo("Return").unwrap().key, 0x0D);
        assert_eq!(parse_combo("return").unwrap().key, 0x0D);
        assert_eq!(
            parse_combo("alt+F4").unwrap(),
            Combo {
                modifiers: vec![VK_MENU],
                key: 0x73
            }
        );
        assert_eq!(parse_combo("BackSpace").unwrap().key, 0x08);
        assert_eq!(
            parse_combo("cmd+plus").unwrap(),
            Combo {
                modifiers: vec![VK_CONTROL],
                key: 0xBB
            }
        );
    }

    #[test]
    fn cmd_is_ctrl_and_super_is_the_windows_key() {
        assert_eq!(parse_combo("cmd+c").unwrap().modifiers, vec![VK_CONTROL]);
        assert_eq!(parse_combo("super+e").unwrap().modifiers, vec![VK_LWIN]);
    }

    #[test]
    fn delete_follows_each_platform() {
        assert_eq!(
            parse_combo("Delete").unwrap().key,
            0x2E,
            "xkb Delete = forward delete"
        );
        assert_eq!(
            parse_combo("delete").unwrap().key,
            0x08,
            "macOS delete = backspace"
        );
        assert_eq!(parse_combo("forwarddelete").unwrap().key, 0x2E);
    }

    #[test]
    fn unknown_names_are_refused() {
        assert!(parse_combo("hyper+a")
            .unwrap_err()
            .contains("unknown modifier"));
        assert!(parse_combo("ctrl+nope")
            .unwrap_err()
            .contains("unknown key"));
        assert!(parse_combo("ctrl+").is_err());
        assert!(parse_combo("fn+a").is_err());
    }

    #[test]
    fn surrogate_pairs_go_out_as_two_units_and_newlines_as_enter() {
        let ev = text_events("a🎉\n");
        assert_eq!(ev.len(), 2 + 4 + 2);
    }

    #[test]
    fn normalisation_hits_pixel_centres_on_a_virtual_desktop_with_negative_origin() {
        // The spike machine: virtual desktop (-1920,-122) 5760x1203.
        let v = (-1920, -122, 5760, 1203);
        assert_eq!(normalise(-1920, -122, v), (0, 0));
        assert_eq!(normalise(3839, 1080, v), (65535, 65535));
    }
}

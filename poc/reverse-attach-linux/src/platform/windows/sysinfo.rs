//! `sys_info` on Windows (minimal): the `host` / `displays` / `permissions` / `agent`
//! keys OpenAB Connect reads, the OS build, and a readable summary.

use serde_json::{json, Value};
use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ};
use windows_sys::Win32::System::SystemInformation::GetTickCount64;

use super::desk;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// A REG_SZ under HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion.
fn current_version(name: &str) -> Option<String> {
    let key = wide(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion");
    let value = wide(name);
    let mut buf = [0u16; 128];
    let mut len = std::mem::size_of_val(&buf) as u32;
    // SAFETY: buffers outlive the call; `len` is the buffer size in bytes.
    let rc = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buf.as_mut_ptr().cast(),
            &mut len,
        )
    };
    if rc != 0 {
        return None;
    }
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..n]))
}

fn os_name() -> String {
    let build: u32 = current_version("CurrentBuild")
        .and_then(|b| b.parse().ok())
        .unwrap_or(0);
    // ProductName still says "Windows 10" on Windows 11; the build number does not lie.
    let family = if build >= 22000 {
        "Windows 11"
    } else {
        "Windows 10"
    };
    match current_version("DisplayVersion") {
        Some(v) => format!("{family} {v} (build {build})"),
        None => format!("{family} (build {build})"),
    }
}

pub(crate) fn tool_sys_info(_: &Value) -> Result<Value, (i64, String)> {
    let host = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "unknown".to_string());
    let user = std::env::var("USERNAME").unwrap_or_default();
    let desktop = desk::input_desktop();
    let usable = matches!(&desktop, Some(n) if n.eq_ignore_ascii_case("Default"));
    let displays: Vec<Value> = desk::displays()
        .iter()
        .enumerate()
        .map(|(i, d)| {
            json!({
                "index": i,
                "kind": "windows",
                "device": d.device,
                "main": d.primary,
                "origin": { "x": d.left, "y": d.top },
                "pixels": { "width": d.width, "height": d.height },
                "scale_percent": d.dpi * 100 / 96,
            })
        })
        .collect();
    let os = os_name();
    let cpu_count = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(0);
    // SAFETY: no arguments.
    let uptime_secs = unsafe { GetTickCount64() } / 1000;

    let structured = json!({
        "host": host,
        "hostname": host,
        "os": os,
        "arch": std::env::consts::ARCH,
        "cpu_count": cpu_count,
        "user": user,
        "gui_session": desktop.is_some(),
        "displays": displays,
        "permissions": {
            "screen_recording": usable,
            "accessibility": usable,
            "input_desktop": desktop.clone().unwrap_or_else(|| "unavailable".to_string()),
        },
        "uptime_secs": uptime_secs,
        "agent": {
            "name": super::SERVER_NAME,
            "version": super::SERVER_VERSION,
            "platform": "windows",
            "pid": std::process::id(),
        }
    });

    let mut lines = vec![
        format!("{host} — {os}, {} CPUs", cpu_count),
        format!(
            "user {user}; input desktop {}",
            desktop.as_deref().unwrap_or("unavailable")
        ),
        format!(
            "displays: {}",
            displays
                .iter()
                .map(|d| format!(
                    "{}×{} px at {}%{}",
                    d["pixels"]["width"],
                    d["pixels"]["height"],
                    d["scale_percent"],
                    if d["main"] == true { " (main)" } else { "" }
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    ];
    if !usable {
        lines.push(
            "→ screenshot, mouse and key will fail until the user is back on the normal desktop"
                .to_string(),
        );
    }
    lines.push(format!(
        "agent {} {}",
        super::SERVER_NAME,
        super::SERVER_VERSION
    ));
    Ok(json!({
        "content": [ { "type": "text", "text": lines.join("\n") } ],
        "structuredContent": structured
    }))
}

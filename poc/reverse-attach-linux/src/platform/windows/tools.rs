//! The tools a Windows node serves (POC). More are added ticket by ticket.

use serde_json::{json, Value};

use super::capture::tool_screenshot;
use super::exec;
use super::input::{tool_key, tool_mouse};
use super::jobs;
use super::sysinfo::tool_sys_info;
use crate::mcp::ToolClass;
use crate::tools::LocalTool;

pub(crate) static LOCAL_TOOLS: &[LocalTool] = &[
    LocalTool {
        name: "sys_info",
        class: ToolClass::Observe,
        listing: sys_info_listing,
        call: tool_sys_info,
    },
    LocalTool {
        name: "screenshot",
        class: ToolClass::Observe,
        listing: screenshot_listing,
        call: tool_screenshot,
    },
    LocalTool {
        name: "mouse",
        class: ToolClass::Shell, // can open a terminal
        listing: mouse_listing,
        call: tool_mouse,
    },
    LocalTool {
        name: "key",
        class: ToolClass::Shell, // can type into one
        listing: key_listing,
        call: tool_key,
    },
    LocalTool {
        name: "powershell",
        class: ToolClass::Shell,
        listing: exec::listing,
        call: exec::tool_powershell,
    },
    LocalTool {
        name: "exec_start",
        class: ToolClass::Shell,
        listing: exec_start_listing,
        call: jobs::tool_exec_start,
    },
    LocalTool {
        name: "exec_poll",
        class: ToolClass::Observe,
        listing: exec_poll_listing,
        call: jobs::tool_exec_poll,
    },
    LocalTool {
        name: "exec_list",
        class: ToolClass::Observe,
        listing: exec_list_listing,
        call: jobs::tool_exec_list,
    },
    LocalTool {
        name: "exec_cancel",
        class: ToolClass::Act,
        listing: exec_cancel_listing,
        call: jobs::tool_exec_cancel,
    },
];

fn exec_start_listing() -> Value {
    json!({
        "name": "exec_start",
        "description": "Start a PowerShell command in the background and return a job_id immediately, \
                        for work that outlives one request. Poll it with exec_poll, stop it with \
                        exec_cancel. Same shell/session as powershell. timeout_secs 0 = no timeout.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "command": { "type": "string" },
                "cwd": { "type": "string", "description": "leading ~ expands to %USERPROFILE%" },
                "timeout_secs": { "type": "integer", "description": "0 = no timeout (stop via exec_cancel). Default 0." },
                "env": { "type": "object", "additionalProperties": { "type": "string" } }
            },
            "required": ["command"],
            "additionalProperties": false
        }
    })
}

fn exec_poll_listing() -> Value {
    json!({
        "name": "exec_poll",
        "description": "Fetch a background job's state and output since your last poll. Pass job_id and, \
                        for only-new output, the stdout_since/stderr_since byte offsets from the previous \
                        poll. Terminal state (exited/killed) carries exit_code.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "job_id": { "type": "string" },
                "stdout_since": { "type": "integer" },
                "stderr_since": { "type": "integer" }
            },
            "required": ["job_id"],
            "additionalProperties": false
        }
    })
}

fn exec_list_listing() -> Value {
    json!({
        "name": "exec_list",
        "description": "List background jobs: all running plus the 10 most recently finished, each with \
                        job_id, state, pid, exit_code, command, cwd, timestamps and current stdout/stderr sizes.",
        "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
    })
}

fn exec_cancel_listing() -> Value {
    json!({
        "name": "exec_cancel",
        "description": "Stop a running job (or drop a finished one). signal KILL (default, immediate, exit \
                        137) or TERM (best-effort WM_CLOSE, then forced after 5 s, exit 143). Poll once more \
                        for final output.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "job_id": { "type": "string" },
                "signal": { "type": "string", "enum": ["KILL", "TERM"] }
            },
            "required": ["job_id"],
            "additionalProperties": false
        }
    })
}

fn mouse_listing() -> Value {
    json!({
        "name": "mouse",
        "description": "Mouse input on this Windows computer (SendInput). Coordinates are physical \
                        pixels relative to the top-left of `display` (0 = primary) — the same space \
                        as `screenshot` at scale 1. Actions: move, click, double_click, right_click, \
                        drag (x,y → to_x,to_y), scroll (dy/dx in wheel notches, positive = down/right; \
                        the opposite sign of macOS). `modifiers` (e.g. [\"ctrl\"], [\"shift\"]) are held \
                        during a click.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["move", "click", "double_click", "right_click", "drag", "scroll"] },
                "x": { "type": "number" }, "y": { "type": "number" },
                "to_x": { "type": "number" }, "to_y": { "type": "number" },
                "dx": { "type": "number" }, "dy": { "type": "number" },
                "display": { "type": "integer", "description": "display index, 0 = primary" },
                "modifiers": { "type": "array", "items": { "type": "string" }, "description": "held during a click" }
            },
            "required": ["action"],
            "additionalProperties": false
        }
    })
}

fn key_listing() -> Value {
    json!({
        "name": "key",
        "description": "Keyboard input on this Windows computer (SendInput). `type`: send `text` as \
                        Unicode into the focused window (any script, emoji included; newlines become \
                        Enter; the window's IME is paused while typing). `press`: a `combo` such as \
                        \"Return\", \"ctrl+c\", \"alt+F4\", or `keys`, a list run in order. Key names \
                        follow xkb or macOS; `cmd` means ctrl, `super`/`win` is the Windows key. \
                        `delay_ms` paces characters/combos.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["type", "press"] },
                "text": { "type": "string" },
                "combo": { "type": "string" },
                "keys": { "type": "array", "items": { "type": "string" } },
                "delay_ms": { "type": "integer" }
            },
            "required": ["action"],
            "additionalProperties": false
        }
    })
}

fn sys_info_listing() -> Value {
    json!({
        "name": "sys_info",
        "description": "Describe this Windows computer: host, Windows version, user, displays \
                        (pixels, scale, which is main), whether the input desktop is usable \
                        (not locked, no UAC prompt). Call this first to learn what the other \
                        tools can do here.",
        "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
    })
}

fn screenshot_listing() -> Value {
    json!({
        "name": "screenshot",
        "description": "Capture a display of this Windows computer and return it as an image. \
                        `display` 0 = primary. `scale` (default 0.5) is output pixels per display \
                        pixel; at scale 1 an image pixel is exactly a `mouse` coordinate. `region` \
                        {x,y,width,height} in display pixels crops before scaling — use it with \
                        scale ≥ 1 to read small text. `format` png (default) or jpeg with \
                        `quality` 1–100 or 0–1. Fails while the screen is locked or a UAC prompt \
                        is up.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "display": { "type": "integer", "description": "display index, 0 = primary" },
                "scale":   { "type": "number", "description": "output scale factor 0.05–2, default 0.5" },
                "region":  {
                    "type": "object",
                    "description": "crop rectangle in display pixels",
                    "properties": {
                        "x": { "type": "number" }, "y": { "type": "number" },
                        "width": { "type": "number" }, "height": { "type": "number" }
                    },
                    "required": ["x", "y", "width", "height"]
                },
                "format":  { "type": "string", "enum": ["jpeg", "png"], "description": "default png" },
                "quality": { "type": "number", "description": "jpeg quality 1–100 (or 0–1), default 80" }
            },
            "additionalProperties": false
        }
    })
}

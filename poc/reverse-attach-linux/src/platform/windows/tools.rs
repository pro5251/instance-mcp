//! The tools a Windows node serves (POC). More are added ticket by ticket.

use serde_json::{json, Value};

use super::capture::tool_screenshot;
use super::exec;
use super::input::{tool_key, tool_mouse};
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
];

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

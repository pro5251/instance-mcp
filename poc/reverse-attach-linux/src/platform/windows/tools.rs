//! The tools a Windows node serves (POC). More are added ticket by ticket.

use serde_json::{json, Value};

use super::capture::tool_screenshot;
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
];

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

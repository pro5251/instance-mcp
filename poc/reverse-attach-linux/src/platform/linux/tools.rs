//! The tools a Linux hands node serves, with their `tools/list` entries. The
//! listings are pinned by `tests/golden/`; change them on purpose only.

use serde_json::{json, Value};

use crate::mcp::ToolClass;
use crate::tools::bash::tool_bash;
use crate::tools::input::{tool_key, tool_mouse};
use crate::tools::screen::tool_screenshot;
use crate::tools::sysinfo::tool_sys_info;
use crate::tools::{tool_result, LocalTool};

// owner and desktop get every local tool, `bash` included: `desktop` is not a boundary
// on any platform (instance-mcp#45) — mouse and keyboard reach a terminal, so hiding
// `bash` would remove a convenience, not a privilege. `observe` is the real boundary:
// sys_info + screenshot only.
pub(crate) static LOCAL_TOOLS: &[LocalTool] = &[
    LocalTool {
        name: "sys_info",
        class: ToolClass::Observe,
        listing: sys_info_listing,
        call: |_| Ok(tool_result(tool_sys_info())),
    },
    LocalTool {
        name: "screenshot",
        class: ToolClass::Observe,
        listing: screenshot_listing,
        call: tool_screenshot,
    },
    LocalTool {
        name: "bash",
        class: ToolClass::Shell,
        listing: bash_listing,
        call: tool_bash,
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
];

fn sys_info_listing() -> Value {
    json!({
        "name": "sys_info",
        "description": "Report OS, CPU, memory, architecture and hostname of this node.",
        "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
    })
}

fn screenshot_listing() -> Value {
    json!({
        "name": "screenshot",
        "description": "Capture the node's Wayland display (via grim) and return it as an image. \
                        Default PNG at scale 0.5 (960x540 for a 1080p output). jpeg only if the \
                        node's grim was built with JPEG support (Debian's is not).",
        "inputSchema": {
            "type": "object",
            "properties": {
                "scale":   { "type": "number", "description": "output scale factor, default 0.5" },
                "format":  { "type": "string", "enum": ["jpeg", "png"], "description": "default png" },
                "quality": { "type": "integer", "description": "jpeg quality 1-100, default 80" }
            },
            "additionalProperties": false
        }
    })
}

fn bash_listing() -> Value {
    json!({
        "name": "bash",
        "description": "Run a command with `bash -c` on this node as the daemon user. Returns stdout, \
                        stderr, exit code and duration. The whole process group is killed on timeout \
                        (exit 137, timed_out=true). Output is capped per stream.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "command":          { "type": "string" },
                "cwd":              { "type": "string", "description": "working directory; leading ~ expands to $HOME" },
                "timeout_secs":     { "type": "integer", "description": "default 60, max 600" },
                "max_output_bytes": { "type": "integer", "description": "per stream, default 65536, max 1048576" }
            },
            "required": ["command"],
            "additionalProperties": false
        }
    })
}

fn mouse_listing() -> Value {
    json!({
        "name": "mouse",
        "description": "Pointer input on this node's Wayland display (wlroots virtual pointer). Coordinates \
                        are display pixels = screenshot pixels at scale 1 (1920x1080 here). Actions: move, \
                        click, double_click, right_click, drag (x,y → to_x,to_y), scroll (dy/dx in wheel \
                        notches/lines, positive = down/right).",
        "inputSchema": {
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["move", "click", "double_click", "right_click", "drag", "scroll"] },
                "x": { "type": "number" }, "y": { "type": "number" },
                "to_x": { "type": "number" }, "to_y": { "type": "number" },
                "dx": { "type": "number" }, "dy": { "type": "number" }
            },
            "required": ["action"],
            "additionalProperties": false
        }
    })
}

fn key_listing() -> Value {
    json!({
        "name": "key",
        "description": "Keyboard input via wtype. `type`: send text (unicode, layout independent). `press`: \
                        a key combo such as \"Return\", \"Tab\", \"ctrl+c\", \"ctrl+shift+t\", \"alt+F4\" \
                        (xkb key names; modifiers ctrl/shift/alt/super).",
        "inputSchema": {
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["type", "press"] },
                "text": { "type": "string" },
                "combo": { "type": "string" }
            },
            "required": ["action"],
            "additionalProperties": false
        }
    })
}

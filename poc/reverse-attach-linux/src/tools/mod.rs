//! Local tools. Each returns the MCP tool-result shape via `tool_result`.
//!
//! Which tools a node serves is decided by its platform (`platform::LOCAL_TOOLS`):
//! every entry carries its own boundary class, so a tool cannot be served without
//! being classified.

#[cfg(unix)]
pub mod bash;
pub mod input;
pub mod screen;
#[cfg(target_os = "linux")]
pub mod sysinfo;

use serde_json::{json, Value};

use crate::mcp::ToolClass;

/// A local tool as a platform registers it.
pub(crate) struct LocalTool {
    pub(crate) name: &'static str,
    pub(crate) class: ToolClass,
    /// The `tools/list` entry (name, description, inputSchema).
    pub(crate) listing: fn() -> Value,
    pub(crate) call: fn(&Value) -> Result<Value, (i64, String)>,
}

/// This node's local tools, in `tools/list` order.
pub(crate) fn local_tools() -> &'static [LocalTool] {
    crate::platform::LOCAL_TOOLS
}

pub(crate) fn local_tool(name: &str) -> Option<&'static LocalTool> {
    local_tools().iter().find(|t| t.name == name)
}

// Wrap a structured value into the MCP tool result shape.
pub fn tool_result(structured: Value) -> Value {
    let text = serde_json::to_string_pretty(&structured).unwrap_or_else(|_| structured.to_string());
    json!({
        "content": [ { "type": "text", "text": text } ],
        "structuredContent": structured
    })
}

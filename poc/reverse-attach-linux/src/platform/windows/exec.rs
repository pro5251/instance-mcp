//! `powershell`: run a command in Windows PowerShell 5.1 to completion. Same argument
//! limits and result shape as the Linux `bash` tool (so callers and Connect see one
//! contract), with Windows specifics from spike 3 (spec §4.4).

use std::time::Duration;

use serde_json::{json, Value};

use super::proc;
use crate::tools::tool_result;

fn clamp_u64(args: &Value, key: &str, default: u64, lo: u64, hi: u64) -> u64 {
    args.get(key)
        .and_then(Value::as_u64)
        .unwrap_or(default)
        .clamp(lo, hi)
}

/// Expand a leading `~` to `%USERPROFILE%` and require the directory to exist.
pub(crate) fn resolve_cwd(raw: &str) -> Result<String, String> {
    let path = if let Some(rest) = raw.strip_prefix('~') {
        let home =
            std::env::var("USERPROFILE").map_err(|_| "USERPROFILE is not set".to_string())?;
        format!("{home}{rest}")
    } else {
        raw.to_string()
    };
    if std::path::Path::new(&path).is_dir() {
        Ok(path)
    } else {
        Err(format!("cwd is not a directory: {path}"))
    }
}

pub(crate) fn tool_powershell(args: &Value) -> Result<Value, (i64, String)> {
    let bad = |e: String| (-32602, e);
    let command = args
        .get("command")
        .and_then(Value::as_str)
        .filter(|c| !c.is_empty())
        .ok_or_else(|| bad("command is required".to_string()))?;
    if command.chars().count() > proc::MAX_COMMAND_CHARS {
        return Err(bad(format!(
            "command too long ({} chars; max {})",
            command.chars().count(),
            proc::MAX_COMMAND_CHARS
        )));
    }
    let timeout = Duration::from_secs(clamp_u64(args, "timeout_secs", 60, 1, 600));
    let cap = clamp_u64(args, "max_output_bytes", 65_536, 1, 1_048_576) as usize;
    let cwd = match args.get("cwd").and_then(Value::as_str) {
        Some(c) => Some(resolve_cwd(c).map_err(bad)?),
        None => None,
    };
    let env = args.get("env").cloned().unwrap_or(Value::Null);

    let spawned = proc::spawn(command, cwd.as_deref(), &env).map_err(|e| (-32000, e))?;
    let out = proc::run(spawned, timeout, cap);

    Ok(tool_result(json!({
        "exit_code": out.exit_code,
        "stdout": String::from_utf8_lossy(&out.stdout),
        "stderr": String::from_utf8_lossy(&out.stderr),
        "stdout_truncated": out.stdout_truncated,
        "stderr_truncated": out.stderr_truncated,
        "timed_out": out.timed_out,
        "duration_ms": out.duration_ms,
        "pid": out.pid
    })))
}

pub(crate) fn listing() -> Value {
    json!({
        "name": "powershell",
        "description": "Run a command in Windows PowerShell 5.1 (-NoProfile -NonInteractive) on this \
                        computer as the desktop user. Returns stdout, stderr, exit code and duration. \
                        Output is UTF-8. The whole process tree is killed on timeout (exit 137, \
                        timed_out=true); GUI apps the command starts (e.g. Start-Process) keep running \
                        after it returns. Output is capped per stream. Scripts (.ps1) may be blocked by \
                        the machine's execution policy; a single command passed here is not.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "command":          { "type": "string" },
                "cwd":              { "type": "string", "description": "working directory; leading ~ expands to %USERPROFILE%" },
                "timeout_secs":     { "type": "integer", "description": "default 60, max 600" },
                "max_output_bytes": { "type": "integer", "description": "per stream, default 65536, max 1048576" },
                "env":              { "type": "object", "additionalProperties": { "type": "string" }, "description": "extra environment variables" }
            },
            "required": ["command"],
            "additionalProperties": false
        }
    })
}

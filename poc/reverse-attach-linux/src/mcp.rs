//! MCP JSON-RPC dispatch (`initialize`, `tools/list`, `tools/call`), the local tool table, and re-serving an upstream MCP (e.g. @playwright/mcp) filtered by profile.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

use crate::http::{http_post, parse_mcp_body, HttpReply};
use crate::tools::{local_tool, local_tools};

// Upstream MCP (e.g. @playwright/mcp on loopback), re-served under our tools/list.
// Mirrors Swift `UpstreamMCP`: Streamable HTTP request/response, session id held
// here and re-established on 400/404, tools cached 30 s, down → tools absent.
// ---------------------------------------------------------------------------

pub(crate) struct Upstream {
    pub(crate) name: String,
    pub(crate) url: String,
    pub(crate) session_id: Mutex<Option<String>>,
    pub(crate) cache: Mutex<Option<(std::time::Instant, Vec<Value>)>>,
}

impl Upstream {
    /// MCP_UPSTREAM="browser=http://127.0.0.1:8794/mcp[,name=url...]"
    pub(crate) fn from_env() -> Vec<Arc<Upstream>> {
        std::env::var("MCP_UPSTREAM")
            .unwrap_or_default()
            .split(',')
            .filter_map(|e| e.trim().split_once('='))
            .map(|(n, u)| {
                Arc::new(Upstream {
                    name: n.trim().to_string(),
                    url: u.trim().to_string(),
                    session_id: Mutex::new(None),
                    cache: Mutex::new(None),
                })
            })
            .collect()
    }

    fn post(&self, msg: &Value) -> Result<HttpReply, String> {
        let sid = self.session_id.lock().ok().and_then(|g| g.clone());
        let mut headers: Vec<(&str, &str)> = vec![
            ("Content-Type", "application/json"),
            ("Accept", "application/json, text/event-stream"),
        ];
        if let Some(s) = sid.as_deref() {
            headers.push(("Mcp-Session-Id", s));
        }
        http_post(
            &self.url,
            &headers,
            &msg.to_string(),
            Duration::from_secs(90),
        )
    }

    fn initialize(&self) -> Result<(), String> {
        // Never send a stale id on initialize: the upstream answers 404 to it.
        if let Ok(mut g) = self.session_id.lock() {
            *g = None;
        }
        let r = self.post(&json!({
            "jsonrpc": "2.0", "id": 0, "method": "initialize",
            "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                       "clientInfo": {"name": "instance-mcp-rpi", "version": "0.4.0"}}
        }))?;
        if !(200..300).contains(&r.status) {
            return Err(format!(
                "upstream {} initialize HTTP {}",
                self.name, r.status
            ));
        }
        let sid = r
            .headers
            .iter()
            .find(|(k, _)| k == "mcp-session-id")
            .map(|(_, v)| v.clone());
        if let Ok(mut g) = self.session_id.lock() {
            *g = sid;
        }
        let _ = self.post(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        if let Ok(mut c) = self.cache.lock() {
            *c = None;
        }
        Ok(())
    }

    pub(crate) fn rpc(&self, method: &str, params: Option<Value>) -> Result<Value, String> {
        if self
            .session_id
            .lock()
            .ok()
            .map(|g| g.is_none())
            .unwrap_or(true)
        {
            self.initialize()?;
        }
        let mut msg = json!({"jsonrpc": "2.0", "id": 1, "method": method});
        if let Some(p) = &params {
            msg["params"] = p.clone();
        }
        let mut r = self.post(&msg)?;
        if r.status == 404 || r.status == 400 {
            // Upstream lost our session (restart). One re-init, one retry.
            self.initialize()?;
            r = self.post(&msg)?;
        }
        if !(200..300).contains(&r.status) {
            return Err(format!(
                "upstream {} HTTP {}: {}",
                self.name,
                r.status,
                &r.body[..r.body.len().min(120)]
            ));
        }
        let v = parse_mcp_body(&r.body)?;
        if let Some(e) = v.get("error") {
            return Err(format!("upstream {} error: {e}", self.name));
        }
        Ok(v.get("result").cloned().unwrap_or(Value::Null))
    }

    pub(crate) fn tools(&self) -> Vec<Value> {
        if let Ok(c) = self.cache.lock() {
            if let Some((at, t)) = c.as_ref() {
                if at.elapsed() < Duration::from_secs(30) {
                    return t.clone();
                }
            }
        }
        let list = match self.rpc("tools/list", None) {
            Ok(r) => r
                .get("tools")
                .and_then(|t| t.as_array())
                .cloned()
                .unwrap_or_default(),
            Err(e) => {
                eprintln!("upstream {}: {e}", self.name);
                // Do not cache a failure: the next tools/list retries immediately.
                if let Ok(mut c) = self.cache.lock() {
                    *c = None;
                }
                return Vec::new();
            }
        };
        if let Ok(mut c) = self.cache.lock() {
            *c = Some((std::time::Instant::now(), list.clone()));
        }
        list
    }
}

/// Same list as Swift `ToolProfile.desktopBrowserTools`: navigate / read / interact.
/// Everything else from the upstream (evaluate / run_code_unsafe = arbitrary JS, upload,
/// pdf, network, raw mouse-by-coordinate, dialogs, close, and any new tool) is denied
/// under `desktop`.
pub(crate) const DESKTOP_BROWSER_TOOLS: &[&str] = &[
    "browser_click",
    "browser_console_messages",
    "browser_fill_form",
    "browser_find",
    "browser_hover",
    "browser_navigate",
    "browser_navigate_back",
    "browser_press_key",
    "browser_resize",
    "browser_select_option",
    "browser_snapshot",
    "browser_tabs",
    "browser_take_screenshot",
    "browser_type",
    "browser_wait_for",
];

pub(crate) fn upstream_tool_allowed(name: &str, profile: &str) -> bool {
    match normalize_profile(profile) {
        Some("owner") => true,
        Some("desktop") => DESKTOP_BROWSER_TOOLS.contains(&name),
        // observe: look only; no upstream (browser) tool is an observation of this node.
        _ => false,
    }
}

/// Local tools `observe` may call: look, never act (instance-mcp#45).
pub(crate) const OBSERVE_TOOLS: &[&str] = &["sys_info", "screenshot"];

/// What a tool can do, as far as a boundary is concerned (same classes as the
/// Swift `ToolProfile.ToolClass`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ToolClass {
    /// Reads only; changes nothing.
    Observe,
    /// Changes state, but cannot run code as the desktop user.
    #[allow(dead_code)]
    Act,
    /// Reaches the desktop user's shell, directly or by driving the GUI.
    #[cfg_attr(not(test), allow(dead_code))]
    Shell,
}

/// The boundary class of a local tool; every registered tool carries one
/// (`tools::LocalTool::class`), so none can be served unclassified.
pub(crate) fn local_tool_class(name: &str) -> Option<ToolClass> {
    local_tool(name).map(|t| t.class)
}

/// Whether a profile honestly grants the node user's shell (instance-mcp#45).
#[cfg(test)]
pub(crate) fn profile_is_shell_equivalent(profile: &str) -> bool {
    matches!(normalize_profile(profile), Some("owner") | Some("desktop"))
}

/// Whether a local tool is visible/callable under `profile`.
pub(crate) fn local_tool_allowed(name: &str, profile: &str) -> bool {
    match normalize_profile(profile) {
        Some("owner") | Some("desktop") => true,
        // Both: on the allowlist *and* classified as observation. A tool added to the
        // allowlist by mistake is still refused unless it was also classified as
        // changing nothing.
        Some("observe") => {
            OBSERVE_TOOLS.contains(&name) && local_tool_class(name) == Some(ToolClass::Observe)
        }
        _ => false,
    }
}

/// `owner` | `desktop` | `observe`. The old name `sandbox` is refused like any unknown
/// profile (instance-mcp#45): it promised a boundary that does not exist.
pub(crate) fn normalize_profile(profile: &str) -> Option<&'static str> {
    match profile {
        "owner" => Some("owner"),
        "desktop" => Some("desktop"),
        "observe" => Some("observe"),
        _ => None,
    }
}

pub(crate) static UPSTREAMS: Mutex<Vec<Arc<Upstream>>> = Mutex::new(Vec::new());

pub(crate) fn upstreams() -> Vec<Arc<Upstream>> {
    UPSTREAMS.lock().map(|g| g.clone()).unwrap_or_default()
}

/// Upstream tools visible to `profile`, excluding names that collide with local tools.
pub(crate) fn upstream_tools_for(
    profile: &str,
    local_names: &[&str],
) -> Vec<(Arc<Upstream>, Value)> {
    let mut out = Vec::new();
    for up in upstreams() {
        for t in up.tools() {
            let Some(name) = t.get("name").and_then(|n| n.as_str()) else {
                continue;
            };
            if local_names.contains(&name) || !upstream_tool_allowed(name, profile) {
                continue;
            }
            out.push((up.clone(), t));
        }
    }
    out
}

pub(crate) fn upstream_owning(name: &str, profile: &str) -> Option<Arc<Upstream>> {
    if !upstream_tool_allowed(name, profile) {
        return None;
    }
    upstreams().into_iter().find(|up| {
        up.tools()
            .iter()
            .any(|t| t.get("name").and_then(|n| n.as_str()) == Some(name))
    })
}

// ---------------------------------------------------------------------------
// answer(): MCP JSON-RPC dispatch
// ---------------------------------------------------------------------------

pub(crate) fn answer(text: &str, profile: &str) -> Option<String> {
    answer_with(text, profile, crate::platform::SERVER_INSTRUCTIONS)
}

/// Like `answer`, but with an explicit initialize `instructions` string (switchboard mode
/// prepends its own; reverse attach and direct `/mcp` pass the platform default).
pub(crate) fn answer_with(text: &str, profile: &str, instructions: Option<&str>) -> Option<String> {
    let req: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(_) => return None,
    };

    // Notifications have no id -> no response.
    let id = req.get("id").cloned()?;

    let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let params = req.get("params").cloned().unwrap_or(Value::Null);

    let result: Result<Value, (i64, String)> = match method {
        "initialize" => {
            let mut result = json!({
                "protocolVersion": "2024-11-05",
                "capabilities": { "tools": {} },
                "serverInfo": {
                    "name": crate::platform::SERVER_NAME,
                    "version": crate::platform::SERVER_VERSION
                }
            });
            if let Some(text) = instructions {
                result["instructions"] = json!(text);
            }
            Ok(result)
        }
        "tools/list" => Ok(json!({ "tools": tool_list(profile) })),
        "tools/call" => handle_tool_call(&params, profile),
        other => Err((-32601, format!("method not found: {other}"))),
    };

    let response = match result {
        Ok(res) => json!({ "jsonrpc": "2.0", "id": id, "result": res }),
        Err((code, message)) => json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": code, "message": message }
        }),
    };

    Some(response.to_string())
}

/// Names of this node's local tools, in `tools/list` order.
pub(crate) fn local_tool_names() -> Vec<&'static str> {
    local_tools().iter().map(|t| t.name).collect()
}

pub(crate) fn tool_list(profile: &str) -> Value {
    let mut tools: Vec<Value> = local_tools()
        .iter()
        .filter(|t| local_tool_allowed(t.name, profile))
        .map(|t| (t.listing)())
        .collect();
    for (_, t) in upstream_tools_for(profile, &local_tool_names()) {
        tools.push(t);
    }
    Value::Array(tools)
}

pub(crate) fn handle_tool_call(params: &Value, profile: &str) -> Result<Value, (i64, String)> {
    let name = params
        .get("name")
        .and_then(|n| n.as_str())
        .ok_or_else(|| (-32602, "missing tool name".to_string()))?;
    let arguments = params.get("arguments").cloned().unwrap_or(Value::Null);

    // A tool hidden from this profile's list is unknown here too, indistinguishable
    // from one that never existed (same rule as the Swift `scoped(to:)`).
    if let Some(tool) = local_tool(name) {
        if !local_tool_allowed(name, profile) {
            return Err((-32601, format!("unknown tool: {name}")));
        }
        return (tool.call)(&arguments);
    }
    match upstream_owning(name, profile) {
        Some(up) => up
            .rpc(
                "tools/call",
                Some(json!({"name": name, "arguments": arguments})),
            )
            .map_err(|e| (-32000, e)),
        None => Err((-32601, format!("unknown tool: {name}"))),
    }
}

// ---------------------------------------------------------------------------

/// The Linux MCP surface as callers see it, pinned byte-for-byte so a refactor or a
/// feature added for another platform cannot change it unnoticed. Regenerate only on
/// purpose: `UPDATE_GOLDEN=1 cargo test golden`, then review the diff.
#[cfg(all(test, target_os = "linux"))]
mod golden_tests {
    use super::*;

    fn check(name: &str, request: &str, profile: &str) {
        let reply = answer(request, profile).expect("a request with an id gets a reply");
        let got: Value = serde_json::from_str(&reply).expect("reply is JSON");
        let got = serde_json::to_string_pretty(&got).expect("serialises") + "\n";
        let path = format!("{}/tests/golden/{name}.json", env!("CARGO_MANIFEST_DIR"));
        if std::env::var_os("UPDATE_GOLDEN").is_some() {
            std::fs::write(&path, &got).expect("write golden");
        }
        let want = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("{path}: {e} (run with UPDATE_GOLDEN=1 to create)"));
        assert_eq!(got, want, "{name} drifted from {path}");
    }

    #[test]
    fn golden_initialize() {
        check(
            "initialize",
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
            "owner",
        );
    }

    #[test]
    fn golden_tools_list_per_profile() {
        for profile in ["owner", "desktop", "observe"] {
            check(
                &format!("tools_list_{profile}"),
                r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
                profile,
            );
        }
    }

    #[test]
    fn golden_observe_refuses_a_forced_shell_call() {
        check(
            "observe_forced_bash",
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"bash","arguments":{"command":"id"}}}"#,
            "observe",
        );
    }
}

#[cfg(test)]
mod profile_tests {
    use super::*;

    #[test]
    fn unknown_profiles_including_the_old_sandbox_name_do_not_widen() {
        assert_eq!(normalize_profile("owner"), Some("owner"));
        assert_eq!(normalize_profile("desktop"), Some("desktop"));
        assert_eq!(normalize_profile("sandbox"), None);
        assert_eq!(normalize_profile("Sandbox"), None);
        assert_eq!(normalize_profile("observe"), Some("observe"));
        assert_eq!(normalize_profile("browser"), None);
    }

    #[test]
    fn desktop_never_gets_arbitrary_javascript() {
        for tool in ["browser_evaluate", "browser_run_code_unsafe"] {
            assert!(!upstream_tool_allowed(tool, "desktop"), "{tool}");
            assert!(
                !upstream_tool_allowed(tool, "sandbox"),
                "{tool}: unknown profile gets nothing"
            );
            assert!(upstream_tool_allowed(tool, "owner"), "{tool}");
        }
        assert!(upstream_tool_allowed("browser_navigate", "desktop"));
    }

    /// Local tool names as actually served under `profile` (no upstream in tests).
    fn served(profile: &str) -> Vec<String> {
        tool_list(profile)
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_owned())
            .filter(|n| !n.starts_with("browser_"))
            .collect()
    }

    #[test]
    fn every_served_tool_is_classified_and_the_lists_agree() {
        let served = served("owner");
        let mut sorted_served = served.clone();
        sorted_served.sort();
        let mut names: Vec<String> = local_tool_names().iter().map(|s| s.to_string()).collect();
        names.sort();
        assert_eq!(
            sorted_served, names,
            "tool_list and the platform's LOCAL_TOOLS disagree"
        );
        let unclassified: Vec<_> = served
            .iter()
            .filter(|n| local_tool_class(n).is_none())
            .collect();
        assert!(
            unclassified.is_empty(),
            "classify in LOCAL_TOOL_CLASS: {unclassified:?}"
        );
        let stale: Vec<_> = local_tool_names()
            .into_iter()
            .filter(|n| !served.iter().any(|s| s == n))
            .collect();
        assert!(stale.is_empty(), "registered but not served: {stale:?}");
    }

    #[test]
    fn no_profile_claims_to_be_narrower_than_it_is() {
        for profile in ["owner", "desktop", "observe"] {
            let shell: Vec<String> = served(profile)
                .into_iter()
                .filter(|n| local_tool_class(n) == Some(ToolClass::Shell))
                .collect();
            if profile_is_shell_equivalent(profile) {
                assert!(
                    !shell.is_empty(),
                    "{profile} is marked shell-equivalent; keep it honest"
                );
            } else {
                assert!(
                    shell.is_empty(),
                    "{profile} claims no shell but serves {shell:?}"
                );
                let acting: Vec<String> = served(profile)
                    .into_iter()
                    .filter(|n| local_tool_class(n) != Some(ToolClass::Observe))
                    .collect();
                assert!(
                    acting.is_empty(),
                    "{profile} serves tools that act: {acting:?}"
                );
            }
        }
    }

    #[test]
    fn observe_can_only_look() {
        for tool in ["sys_info", "screenshot"] {
            assert!(local_tool_allowed(tool, "observe"), "{tool}");
        }
        for tool in ["bash", "mouse", "key", "something_new"] {
            assert!(!local_tool_allowed(tool, "observe"), "{tool}");
        }
        for tool in [
            "browser_navigate",
            "browser_snapshot",
            "browser_take_screenshot",
        ] {
            assert!(!upstream_tool_allowed(tool, "observe"), "{tool}");
        }
        assert!(local_tool_allowed("bash", "desktop"));
        assert!(
            !local_tool_allowed("bash", "nonsense"),
            "unknown profiles get nothing"
        );
    }
}

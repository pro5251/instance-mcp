#!/usr/bin/env bash
# Verifies the one-step Windows setup end to end on a real Windows desktop (CurrentUser, no
# admin), entering through scripts/setup-winpoc.ps1 with -ApiBase at a fake GitHub:
# install -> healthy -> token auth -> hardened Scheduled Tasks + token DACL -> browser_* via
# Playwright MCP (needs Node.js) -> Claude Code registration (a `claude` stub first on PATH;
# the real Claude Code config is never touched) -> re-run keeps the token -> opt-outs ->
# uninstall / -Purge. Driven from WSL. The first run downloads Chromium (~150 MB).
set -u
cd "$(dirname "$0")/.."
. windows/setup_fixture.sh
PORT=8796; TOKEN=poc-smoke-$RANDOM
API="-ApiBase $GH -Repo test/winpoc -SkipTailscale"
LOG="$WIN_DIR/stub/claude.log"
cclog(){ tr -d '\r' < "$LOG"; }   # the stub is a .cmd: CRLF lines
uninst(){ setup $API -Uninstall -Purge "$@" >/dev/null; }
trap 'uninst; fixture_cleanup' EXIT
uninst
cc_count(){ local n; n=$(grep -c oab-imcp-winpoc "$CLAUDE_JSON" 2>/dev/null); echo "${n:-0}"; }
cc_before=$(cc_count)

printf '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' > "$WIN_DIR/req.json"
REQ=$(wslpath -w "$WIN_DIR/req.json")
tools(){ curl.exe -s -m 10 -X POST "http://127.0.0.1:$PORT/mcp" -H "Authorization: Bearer $1" --data-binary @"$REQ" \
  | python3 -c 'import json,sys;print(" ".join(t["name"] for t in json.load(sys.stdin)["result"]["tools"]))' 2>/dev/null; }
local_n(){ tr ' ' '\n' <<< "$1" | grep -v '^browser_' | grep -c .; }
browser_n(){ tr ' ' '\n' <<< "$1" | grep -c '^browser_'; }
settings(){ psc "\$s=(Get-ScheduledTask -TaskName $1 -TaskPath '\\OpenAB-POC\\').Settings; \"\$(\$s.ExecutionTimeLimit)|\$(\$s.DisallowStartIfOnBatteries)|\$(\$s.MultipleInstances)\""; }
health(){ [[ "$(curl.exe -s -m 3 "http://127.0.0.1:$PORT/healthz")" == ok ]]; }

echo "== install (browser + Claude Code) =="
[[ -n "$NODE_DIR" ]] || echo "note: no Node.js on Windows; the browser checks will fail"
: > "$LOG"
setup $API -Token "$TOKEN"
echo "$out" | sed -n '/==== ready ====/,/^=====/p'
check "setup exits 0" "[[ $rc == 0 ]]"
check "installer reports ready with the MCP URL" "has 'MCP URL' && has '======'"
check "the node task exists" "[[ '$(task_state oab-imcp-winpoc)' =~ ^(Ready|Running)$ ]]"
check "the Playwright task exists" "[[ '$(task_state oab-imcp-winpoc-pw)' =~ ^(Ready|Running)$ ]]"
check "healthz is ok" "health"
TL=""; for _ in $(seq 1 30); do TL=$(tools "$TOKEN"); [[ $(browser_n "$TL") -gt 0 ]] && break; sleep 1; done
check "serves the 9 local tools with the bearer token" "[[ $(local_n "$TL") == 9 ]]"
check "serves browser_* from Playwright MCP" "[[ $(browser_n "$TL") -gt 0 && ' $TL ' == *' browser_navigate '* ]]"
check "a wrong token is rejected (401)" "[[ '$(curl.exe -s -o /dev/null -w '%{http_code}' -X POST "http://127.0.0.1:$PORT/mcp" -H 'Authorization: Bearer WRONG' --data-binary @"$REQ")' == 401 ]]"
check "summary reports browser tools" "has 'browser : browser_* via Playwright MCP on 127.0.0.1:8797'"

echo "== hardened tasks =="
for t in oab-imcp-winpoc oab-imcp-winpoc-pw; do
  s=$(settings $t); echo "$t settings: $s"
  check "$t: no execution time limit (PT0S)" "[[ '$s' == PT0S* ]]"
  check "$t: runs on battery" "[[ '$s' == *'|False|'* ]]"
  check "$t: ignores a second instance" "[[ '$s' == *'|IgnoreNew' ]]"
done
check "node task forwards browser_* (--upstream, a flag not an env var)" "[[ '$(task_args oab-imcp-winpoc)' == *'--upstream browser=http://127.0.0.1:8797/mcp'* ]]"
pa=$(task_args oab-imcp-winpoc-pw)
check "Playwright task runs pw-mcp.ps1 in a hidden PowerShell" "[[ '$(task_exec oab-imcp-winpoc-pw)' == *powershell.exe && '$pa' == *'-WindowStyle Hidden'* && '$pa' == *'imcp-winpoc\\pw-mcp.ps1'* ]]"
check "@playwright/mcp is pinned to 0.0.82 (--save-exact)" "grep -qE '\"@playwright/mcp\": ?\"0\.0\.82\"' '$DATA/pw-mcp/package.json'"

echo "== token DACL =="
acl=$(icacls.exe "$(wslpath -w "$DATA/token")" 2>/dev/null | tr -d '\r')
check "token DACL grants the owner and SYSTEM" "grep -qi 'SYSTEM:' <<< '$acl' && grep -qiE '\\\\[a-z0-9_.-]+:' <<< '$acl'"
check "token DACL excludes Users/Everyone" "! grep -qiE 'BUILTIN..Users:|Everyone:|Authenticated Users:' <<< '$acl'"

echo "== Claude Code registration =="
cclog
check "claude called remove -> add -> get, in that order" "[[ \$(cclog | cut -d' ' -f1-2 | tr '\n' ,) == 'mcp remove,mcp add,mcp get,' ]]"
check "remove targets the POC name in user scope" "cclog | grep -qx 'mcp remove --scope user oab-imcp-winpoc'"
check "add: http, user scope, POC name, loopback URL, bearer header" "cclog | grep -qx 'mcp add --transport http --scope user oab-imcp-winpoc http://127.0.0.1:8796/mcp --header \"Authorization: Bearer $TOKEN\"'"
check "get confirms the POC name" "cclog | grep -qx 'mcp get oab-imcp-winpoc'"
check "summary: registered + the plain-text token note" "has \"Claude Code: registered 'oab-imcp-winpoc' (user scope, http://127.0.0.1:8796/mcp)\" && has 'plain text'"
check "the real Claude Code config was not touched" "[[ \$(cc_count) == $cc_before ]]"

echo "== re-run = update (token kept, no duplicate registration) =="
: > "$LOG"
setup $API
check "re-run exits 0 and is healthy" "[[ $rc == 0 ]] && health"
check "re-run keeps the token" "[[ \$(cat '$DATA/token') == '$TOKEN' ]] && ! has 'generated a bearer token'"
check "re-run replaces the registration (remove before add)" "[[ \$(cclog | cut -d' ' -f1-2 | tr '\n' ,) == 'mcp remove,mcp add,mcp get,' ]]"
CLAUDE_STUB_GET_RC=1 setup $API
check "a failed 'claude mcp get' only warns" "[[ $rc == 0 ]] && has 'not confirmed'"

echo "== uninstall (keep data) =="
: > "$LOG"
setup $API -Uninstall
check "uninstall exits 0" "[[ $rc == 0 ]]"
check "both tasks are gone" "[[ -z '$(task_state oab-imcp-winpoc)' && -z '$(task_state oab-imcp-winpoc-pw)' ]]"
check "uninstall removes the Claude Code registration" "cclog | grep -qx 'mcp remove --scope user oab-imcp-winpoc'"
check "uninstall keeps token and browser install without -Purge" "[[ -f '$DATA/token' && -d '$DATA/pw-mcp' ]]"
check "the node is down" "! health"

echo "== -NoBrowser -NoClaudeCode =="
: > "$LOG"
setup $API -NoBrowser -NoClaudeCode
TL=$(tools "$TOKEN")
check "opt-outs: exits 0, healthy" "[[ $rc == 0 ]] && health"
check "-NoBrowser: no Playwright task" "[[ -z '$(task_state oab-imcp-winpoc-pw)' ]]"
check "-NoBrowser: exactly the 9 local tools" "[[ $(local_n "$TL") == 9 && $(browser_n "$TL") == 0 ]]"
check "-NoBrowser: node task has no --upstream" "[[ '$(task_args oab-imcp-winpoc)' != *--upstream* ]]"
check "-NoClaudeCode: claude never called" "[[ ! -s '$LOG' ]] && has 'Claude Code: skipped (-NoClaudeCode)'"

echo "== no Node.js, no Claude Code =="
SMOKE_PATH="$SYS_PATH" setup $API
check "still installs (exit 0, healthy)" "[[ $rc == 0 ]] && health"
check "no Node.js: browser skipped with the re-run hint" "has 'Node.js not found' && has 're-run setup'"
check "no claude: prints the paste-ready command" "has 'claude mcp add --transport http --scope user oab-imcp-winpoc http://127.0.0.1:8796/mcp --header \"Authorization: Bearer $TOKEN\"'"

echo "== -AllowLogin (no token for loopback) =="
: > "$LOG"
setup $API -NoBrowser -AllowLogin smoke@example.com
check "-AllowLogin: installs" "[[ $rc == 0 ]] && health"
check "-AllowLogin: not registered, says why" "[[ ! -s '$LOG' ]] && has 'Claude Code: not registered' && has 'Tailscale identity'"

echo "== uninstall -Purge =="
: > "$LOG"
setup $API -Uninstall -Purge
check "purge reports program + data removed" "[[ $rc == 0 ]] && has 'removed program + data'"
check "purge deletes the data dir (token, pw-mcp, pw-profile, pw-output)" "[[ ! -d '$DATA' ]]"
check "both tasks are gone after purge" "[[ -z '$(task_state oab-imcp-winpoc)' && -z '$(task_state oab-imcp-winpoc-pw)' ]]"
check "purge removes the Claude Code registration" "cclog | grep -qx 'mcp remove --scope user oab-imcp-winpoc'"
check "the real Claude Code config is still untouched" "[[ \$(cc_count) == $cc_before ]]"

echo "RESULT: $pass passed, $fail failed"
[[ $fail -eq 0 ]]

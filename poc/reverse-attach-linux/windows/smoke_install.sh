#!/usr/bin/env bash
# Verifies the one-step Windows setup end to end on a real Windows desktop (CurrentUser, no
# admin), entering through scripts/setup-winpoc.ps1 with -ApiBase at a fake GitHub:
# install -> healthy -> token auth -> hardened Scheduled Tasks + token DACL -> browser_* via
# Playwright MCP (needs Node.js) -> MCP client registration (Claude Code, Codex, Copilot CLI,
# Gemini CLI via stubs first on PATH; VS Code + Insiders mcp.json in a scratch APPDATA; no
# real client config is touched, only Codex's user env var is really set and then removed)
# -> re-run keeps the token -> opt-outs -> uninstall / -Purge. Driven from WSL. The first
# run downloads Chromium (~150 MB).
set -u
cd "$(dirname "$0")/.."
. windows/setup_fixture.sh
PORT=8796; TOKEN=poc-smoke-$RANDOM
API="-ApiBase $GH -Repo test/winpoc -SkipTailscale"
BEARER="Authorization: Bearer $TOKEN"; URL=http://127.0.0.1:8796/mcp; N=oab-imcp-winpoc
uninst(){ setup $API -Uninstall -Purge "$@" >/dev/null; }
trap 'uninst; fixture_cleanup' EXIT
uninst
real_before=$(real_state)
real_same(){ [[ "$(real_state)" == "$real_before" ]]; }
VS="$APPD/Code/User/mcp.json"; VSI="$APPD/Code - Insiders/User/mcp.json"
# vsget <file> <python expr on d>: read a VS Code mcp.json (written by Windows PowerShell)
vsget(){ python3 -c 'import json,sys;d=json.load(open(sys.argv[1],encoding="utf-8-sig"));print(eval(sys.argv[2]))' "$1" "$2" 2>/dev/null; }
srv(){ vsget "$1" "d['servers']['$N']$2"; }          # srv <file> "['url']"
order(){ clog "$1" | cut -d' ' -f1-2 | tr '\n' ,; }  # verbs the stub saw, in order
logged(){ clog "$1" | grep -qxF -- "$2"; }           # logged <client> <exact argument line>
nolog(){ local c; for c in "$@"; do [[ -s "$WIN_DIR/stub/$c.log" ]] && return 1; done; return 0; }

printf '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' > "$WIN_DIR/req.json"
REQ=$(wslpath -w "$WIN_DIR/req.json")
tools(){ curl.exe -s -m 10 -X POST "http://127.0.0.1:$PORT/mcp" -H "Authorization: Bearer $1" --data-binary @"$REQ" \
  | python3 -c 'import json,sys;print(" ".join(t["name"] for t in json.load(sys.stdin)["result"]["tools"]))' 2>/dev/null; }
local_n(){ tr ' ' '\n' <<< "$1" | grep -v '^browser_' | grep -c .; }
browser_n(){ tr ' ' '\n' <<< "$1" | grep -c '^browser_'; }
settings(){ psc "\$s=(Get-ScheduledTask -TaskName $1 -TaskPath '\\OpenAB-POC\\').Settings; \"\$(\$s.ExecutionTimeLimit)|\$(\$s.DisallowStartIfOnBatteries)|\$(\$s.MultipleInstances)\""; }
health(){ [[ "$(curl.exe -s -m 3 "http://127.0.0.1:$PORT/healthz")" == ok ]]; }

echo "== install (browser + MCP clients) =="
[[ -n "$NODE_DIR" ]] || echo "note: no Node.js on Windows; the browser checks will fail"
clear_logs
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

echo "== MCP client registration =="
for c in claude codex copilot gemini; do echo "-- $c"; clog $c; done
check "claude: remove -> add -> get" "[[ \$(order claude) == 'mcp remove,mcp add,mcp get,' ]]"
check "claude: remove in user scope" "logged claude 'mcp remove --scope user $N'"
check "claude: add http, user scope, loopback URL, bearer header" "logged claude 'mcp add --transport http --scope user $N $URL --header \"$BEARER\"'"
check "claude: get confirms" "logged claude 'mcp get $N'"
check "codex: remove -> add -> get" "[[ \$(order codex) == 'mcp remove,mcp add,mcp get,' ]]"
check "codex: add with --url and the bearer token env var" "logged codex 'mcp add $N --url $URL --bearer-token-env-var $CODEX_VAR'"
check "codex: the token is in the user env var" "[[ \$(user_env $CODEX_VAR) == '$TOKEN' ]]"
check "copilot: remove -> add -> get" "[[ \$(order copilot) == 'mcp remove,mcp add,mcp get,' ]]"
check "copilot: add http, loopback URL, bearer header" "logged copilot 'mcp add --transport http $N $URL --header \"$BEARER\"'"
check "gemini: remove -> add -> list" "[[ \$(order gemini) == 'mcp remove,mcp add,mcp list,' ]]"
check "gemini: remove in user scope" "logged gemini 'mcp remove -s user $N'"
check "gemini: add user scope, http, bearer header, name, URL" "logged gemini 'mcp add -s user --transport http --header \"$BEARER\" $N $URL'"
check "VS Code: user mcp.json created with the server (http, URL)" "[[ \$(srv '$VS' \"['url']\") == $URL && \$(srv '$VS' \"['type']\") == http ]]"
check "VS Code: bearer header in mcp.json" "[[ \$(srv '$VS' \"['headers']['Authorization']\") == 'Bearer $TOKEN' ]]"
check "VS Code Insiders: server added" "[[ \$(srv '$VSI' \"['url']\") == $URL ]]"
check "VS Code Insiders: the user's own server and inputs survive" "[[ \$(vsget '$VSI' \"d['servers']['mine']['url']\") == http://example.invalid/mcp && \$(vsget '$VSI' \"d['inputs']\") == '[]' ]]"
check "VS Code Insiders: non-ASCII text and a one-element array survive" "[[ \$(vsget '$VSI' \"d['servers']['tool']['description']\") == '我的伺服器' && \$(vsget '$VSI' \"d['servers']['tool']['args']\") == \"['x']\" ]]"
check "VS Code Insiders: a backup of the original is kept" "ls '$APPD/Code - Insiders/User/' | grep -q 'mcp.json.*bak'"
check "summary lists every client" "has \"Claude Code: registered '$N'\" && has \"Codex CLI: registered '$N'\" && has \"GitHub Copilot CLI: registered '$N'\" && has \"Gemini CLI: registered '$N'\" && has \"VS Code: registered '$N'\" && has \"VS Code Insiders: registered '$N'\""
check "summary: plain-text note and the Codex env var" "has 'plain text' && has '$CODEX_VAR'"
check "no real client config was touched" "real_same"

echo "== re-run = update (token kept, no duplicate registration) =="
clear_logs
setup $API
check "re-run exits 0 and is healthy" "[[ $rc == 0 ]] && health"
check "re-run keeps the token" "[[ \$(cat '$DATA/token') == '$TOKEN' ]] && ! has 'generated a bearer token'"
check "re-run replaces registrations (remove before add)" "[[ \$(order claude) == 'mcp remove,mcp add,mcp get,' && \$(order codex) == 'mcp remove,mcp add,mcp get,' ]]"
check "re-run: still exactly three servers in Insiders mcp.json" "[[ \$(vsget '$VSI' \"len(d['servers'])\") == 3 ]]"
STUB_GET_RC=1 setup $API -NoBrowser
check "a failed 'mcp get' only warns" "[[ $rc == 0 ]] && has 'not confirmed'"
STUB_ADD_RC=1 setup $API -NoBrowser -Clients codex
check "a failed 'codex mcp add' warns and leaves no token in the environment" "[[ $rc == 0 && -z \$(user_env $CODEX_VAR) ]] && has 'Codex CLI: FAILED'"
printf '// my notes\n{ "servers": {} }\n' > "$VSI"; sum=$(md5sum < "$VSI")
setup $API -NoBrowser
check "unparsable mcp.json (comments) is left alone, with a hint" "[[ $rc == 0 && \$(md5sum < '$VSI') == '$sum' ]] && has 'VS Code Insiders: could not parse'"
seed_insiders

echo "== -Clients / -NoClaudeCode =="
clear_logs
setup $API -NoBrowser -Clients codex,vscode
check "-Clients codex,vscode: only codex is called" "nolog claude copilot gemini && [[ -s '$WIN_DIR/stub/codex.log' ]]"
check "-Clients codex,vscode: VS Code is registered" "[[ \$(srv '$VSI' \"['url']\") == $URL ]]"
setup $API -NoBrowser -Clients bogus
check "-Clients with an unknown name fails clearly" "[[ $rc != 0 ]] && has 'unknown client'"
setup $API -NoBrowser -Clients codex -NoClients
check "-Clients with -NoClients is refused" "[[ $rc != 0 ]] && has 'not both'"
clear_logs
setup $API -NoBrowser -NoClaudeCode
check "-NoClaudeCode excludes only claude" "nolog claude && [[ -s '$WIN_DIR/stub/codex.log' ]]"

echo "== uninstall (keep data) =="
clear_logs
setup $API -Uninstall -Clients codex   # the selection is for install; uninstall cleans every client
check "uninstall exits 0" "[[ $rc == 0 ]]"
check "both tasks are gone" "[[ -z '$(task_state oab-imcp-winpoc)' && -z '$(task_state oab-imcp-winpoc-pw)' ]]"
check "uninstall removes every CLI registration" "logged claude 'mcp remove --scope user $N' && logged codex 'mcp remove $N' && logged copilot 'mcp remove $N' && logged gemini 'mcp remove -s user $N'"
check "uninstall removes the VS Code entries, keeps the user's" "[[ \$(vsget '$VS' \"'$N' in d['servers']\") == False && \$(vsget '$VSI' \"'$N' in d['servers']\") == False && \$(vsget '$VSI' \"'mine' in d['servers']\") == True ]]"
check "uninstall deletes the Codex env var" "[[ -z \$(user_env $CODEX_VAR) ]]"
check "uninstall keeps token and browser install without -Purge" "[[ -f '$DATA/token' && -d '$DATA/pw-mcp' ]]"
check "the node is down" "! health"

echo "== -NoBrowser -NoClients =="
clear_logs
setup $API -NoBrowser -NoClients
TL=$(tools "$TOKEN")
check "opt-outs: exits 0, healthy" "[[ $rc == 0 ]] && health"
check "-NoBrowser: no Playwright task" "[[ -z '$(task_state oab-imcp-winpoc-pw)' ]]"
check "-NoBrowser: exactly the 9 local tools" "[[ $(local_n "$TL") == 9 && $(browser_n "$TL") == 0 ]]"
check "-NoBrowser: node task has no --upstream" "[[ '$(task_args oab-imcp-winpoc)' != *--upstream* ]]"
check "-NoClients: no client called, no env var" "nolog $CLIENTS && [[ -z \$(user_env $CODEX_VAR) ]] && has 'MCP clients: skipped (-NoClients)'"

echo "== no Node.js, no clients =="
SMOKE_PATH="$SYS_PATH" setup $API
check "still installs (exit 0, healthy)" "[[ $rc == 0 ]] && health"
check "no Node.js: browser skipped with the re-run hint" "has 'Node.js not found' && has 're-run setup'"
check "no claude: prints the paste-ready command" "has 'claude mcp add --transport http --scope user $N $URL --header \"$BEARER\"'"
check "no clients: lists what was not found" "has 'not found:' && has 'Codex CLI'"

echo "== -AllowLogin (no token for loopback) =="
clear_logs
setup $API -NoBrowser -AllowLogin smoke@example.com
check "-AllowLogin: installs" "[[ $rc == 0 ]] && health"
check "-AllowLogin: no client registered, says why" "nolog $CLIENTS && has 'MCP clients: not registered' && has 'Tailscale identity'"

echo "== uninstall -Purge =="
setup $API -NoBrowser
clear_logs
setup $API -Uninstall -Purge
check "purge reports program + data removed" "[[ $rc == 0 ]] && has 'removed program + data'"
check "purge deletes the data dir (token, pw-mcp, pw-profile, pw-output)" "[[ ! -d '$DATA' ]]"
check "both tasks are gone after purge" "[[ -z '$(task_state oab-imcp-winpoc)' && -z '$(task_state oab-imcp-winpoc-pw)' ]]"
check "purge removes the registrations and the Codex env var" "logged claude 'mcp remove --scope user $N' && [[ -z \$(user_env $CODEX_VAR) ]]"
check "no real client config was touched, start to end" "real_same"

echo "RESULT: $pass passed, $fail failed"
[[ $fail -eq 0 ]]

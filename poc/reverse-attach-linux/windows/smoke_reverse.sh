#!/usr/bin/env bash
# Windows reverse-attach + grant persistence against the python mock runtime (run in
# WSL; the Windows node reaches it on 127.0.0.1 via WSL2 loopback forwarding). Verifies
# the real hop (Windows node dials the runtime, upgrades, serves MCP), that the grant is
# persisted to a DACL-protected grants.json under %LOCALAPPDATA%, and that it resumes
# after a hard restart within its TTL.
set -u
cd "$(dirname "$0")/.."
EXE_SRC=target/x86_64-pc-windows-gnu/release/reverse-attach.exe
WIN_DIR=/mnt/c/Users/$(cmd.exe /c 'echo %USERNAME%' 2>/dev/null | tr -d '\r')/src/imcp-winpoc
LOCALAPPDATA_WSL=/mnt/c/Users/$(cmd.exe /c 'echo %USERNAME%' 2>/dev/null | tr -d '\r')/AppData/Local
GRANTS="$LOCALAPPDATA_WSL/oab-imcp-winpoc/oab-instance-mcp/grants.json"
PORT=8796; MOCKP=18099
pass=0; fail=0
ok(){ echo "PASS $1"; pass=$((pass+1)); }; bad(){ echo "FAIL $1"; fail=$((fail+1)); }
check(){ if eval "$2"; then ok "$1"; else bad "$1 :: $2"; fi; }
psh(){ env -u PSModulePath powershell.exe -NoProfile -NonInteractive -Command "$1" 2>/dev/null | tr -d '\r'; }

mkdir -p "$WIN_DIR"; cp "$EXE_SRC" "$WIN_DIR/oab-imcp-winpoc.exe"
cmd_kill(){ cmd.exe /c "taskkill /im oab-imcp-winpoc.exe /f" >/dev/null 2>&1; }
cmd_kill; pkill -f mock_runtime.py 2>/dev/null; sleep 0.3
rm -rf "$LOCALAPPDATA_WSL/oab-imcp-winpoc/oab-instance-mcp"
MLOG=/tmp/mock-rev.jsonl; rm -f "$MLOG"
start_node(){ (cd "$WIN_DIR" && ./oab-imcp-winpoc.exe --insecure-local >/tmp/winnode.log 2>&1 &); }
trap 'cmd_kill; pkill -f mock_runtime.py 2>/dev/null' EXIT

PORT=$MOCKP ADMIN=admin-secret CLOSES=1000 SECRETS=pre=preminted LOG=$MLOG python3 mock_runtime.py >/tmp/mock-rev.out 2>&1 &
start_node; sleep 1.5
wcurl(){ curl.exe -s -m 8 "$@"; }  # the node is a Windows process: drive it with Windows curl
attach(){ printf '%s' "$1" > "$WIN_DIR/req.json"; wcurl -X POST "http://127.0.0.1:$PORT/attach" --data-binary @"$(wslpath -w "$WIN_DIR/req.json")"; }
field(){ python3 -c 'import json,sys;print(json.load(sys.stdin).get(sys.argv[1],""))' "$1"; }

echo "== real hop: Windows node dials the runtime =="
r=$(attach '{"runtime":"ws://127.0.0.1:18099","session":"pre","profile":"owner","ttl_secs":300,"secret":"preminted"}')
GID=$(echo "$r" | field id)
check "POST /attach accepted" "[[ -n '$GID' ]]"
for _ in $(seq 1 40); do grep -q '"status": 101' "$MLOG" 2>/dev/null && break; sleep 0.2; done
check "the runtime saw the WS upgrade (101)" "grep -q '\"status\": 101' '$MLOG'"
check "MCP round-tripped over reverse attach (initialize + tools/list)" "grep -q '\"method\": \"tools/list\"' '$MLOG'"
check "Windows sys_info answered over the hop" "grep -q '\"platform\": \"windows\"' '$MLOG'"

echo "== persistence + DACL =="
check "grants.json was written under %LOCALAPPDATA%" "[[ -f '$GRANTS' ]]"
check "the grant is in the file" "grep -q '$GID' '$GRANTS'"
# icacls is a standalone exe (no PowerShell module) and prints the DACL ACEs.
acl=$(icacls.exe "$(wslpath -w "$GRANTS")" 2>/dev/null | tr -d '\r')
echo "grants.json ACEs: $(echo "$acl" | grep -oiE '[A-Za-z0-9_.-]+\\[A-Za-z0-9_. -]+:' | tr '\n' ' ')"
me=$(cmd.exe /c 'echo %USERDOMAIN%\%USERNAME%' 2>/dev/null | tr -d '\r')
check "DACL grants the current user" "grep -qiF '$me:' <<< '$acl'"
check "DACL grants SYSTEM" "grep -qiE 'NT AUTHORITY.{1,3}SYSTEM:' <<< '$acl'"
check "DACL does not grant Users/Everyone/Authenticated Users" "! grep -qiE 'BUILTIN.{1,3}Users:|Everyone:|Authenticated Users:' <<< '$acl'"
check "DACL inheritance is disabled (no inherited ACEs)" "! grep -qE '\\(I\\)' <<< '$acl'"

echo "== resume after a hard restart =="
grep -c '"status": 101' "$MLOG" > /tmp/n1
cmd_kill; sleep 0.6; start_node; sleep 2
r=$(wcurl "http://127.0.0.1:$PORT/attach")
check "GET /attach lists the resumed grant after restart" "[[ '$(echo "$r" | python3 -c 'import json,sys;print(any(g["id"]==sys.argv[1] for g in json.load(sys.stdin)["grants"]))' "$GID" 2>/dev/null)' == True ]]"
for _ in $(seq 1 40); do [[ $(grep -c '"status": 101' "$MLOG") -gt $(cat /tmp/n1) ]] && break; sleep 0.2; done
check "the resumed grant redialed the runtime" "[[ \$(grep -c '\"status\": 101' '$MLOG') -gt \$(cat /tmp/n1) ]]"
check "no attach secret is printed in the node log" "! grep -qi 'preminted' /tmp/winnode.log"

echo "RESULT: $pass passed, $fail failed"
[[ $fail -eq 0 ]]

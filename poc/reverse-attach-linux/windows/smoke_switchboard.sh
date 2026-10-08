#!/usr/bin/env bash
# Windows switchboard mode against a mock openab-sb (WSL python, reached on 127.0.0.1).
# Verifies the node dials /vm/attach, serves observe-scoped MCP with switchboard
# instructions, redials on a 1000 close, and stops on 4002.
set -u
cd "$(dirname "$0")/.."
EXE_SRC=target/x86_64-pc-windows-gnu/release/reverse-attach.exe
WIN_DIR=/mnt/c/Users/$(cmd.exe /c 'echo %USERNAME%' 2>/dev/null | tr -d '\r')/src/imcp-winpoc
SBP=18101
pass=0; fail=0
ok(){ echo "PASS $1"; pass=$((pass+1)); }; bad(){ echo "FAIL $1"; fail=$((fail+1)); }
check(){ if eval "$2"; then ok "$1"; else bad "$1 :: $2"; fi; }
mkdir -p "$WIN_DIR"; cp "$EXE_SRC" "$WIN_DIR/oab-imcp-winpoc.exe"
printf 'sb-secret' > "$WIN_DIR/sb.secret"
SECF=$(wslpath -w "$WIN_DIR/sb.secret")
LOG=/tmp/mock-sb.jsonl
kill_node(){ cmd.exe /c "taskkill /im oab-imcp-winpoc.exe /f" >/dev/null 2>&1; }
MK=""
start_mock(){ rm -f "$LOG"; PORT=$SBP SECRET=sb-secret CLOSES="$1" LOG="$LOG" python3 windows/mock_switchboard.py >/tmp/mock-sb.out 2>&1 & MK=$!; }
stop_all(){ kill_node; [ -n "$MK" ] && kill $MK 2>/dev/null; }
trap stop_all EXIT
start_node(){ (cd "$WIN_DIR" && ./oab-imcp-winpoc.exe --insecure-local \
  --switchboard "ws://127.0.0.1:$SBP/vm/attach" --switchboard-secret-file "sb.secret" \
  --switchboard-profile observe >/tmp/winnode.log 2>&1 &); }

echo "== attach + observe-scoped MCP =="
kill_node; start_mock 1000,1000,1000; start_node; sleep 1.8
for _ in $(seq 1 40); do grep -q '"method": "tools/call"' "$LOG" 2>/dev/null && break; sleep 0.2; done
check "the switchboard saw the WS upgrade (101)" "grep -q '\"status\": 101' '$LOG'"
check "node answered initialize with the POC serverInfo" "grep -q '\"name\": \"oab-imcp-winpoc\"' '$LOG'"
check "initialize instructions mention Switchboard" "grep -q 'OpenAB Switchboard' '$LOG'"
TOOLS=$(python3 -c 'import json,sys;[print(",".join(t["name"] for t in e["reply"]["result"]["tools"])) for e in map(json.loads,open(sys.argv[1])) if e.get("method")=="tools/list"]' "$LOG" 2>/dev/null | head -1)
echo "observe tools: $TOOLS"
check "observe profile serves only sys_info,screenshot" "[[ '$TOOLS' == 'sys_info,screenshot' ]]"
check "sys_info answered over the switchboard (windows)" "grep -q '\"platform\": \"windows\"' '$LOG'"

echo "== redial on a 1000 close =="
for _ in $(seq 1 40); do [[ $(grep -c '"status": 101' "$LOG") -ge 2 ]] && break; sleep 0.3; done
check "node redialled after a 1000 close" "[[ \$(grep -c '\"status\": 101' '$LOG') -ge 2 ]]"

echo "== stop on 4002 (replaced) =="
kill_node; [ -n "$MK" ] && kill $MK 2>/dev/null; sleep 0.3
start_mock 4002; start_node; sleep 2
for _ in $(seq 1 30); do grep -q '"will_close": 4002' "$LOG" 2>/dev/null && break; sleep 0.2; done
n_before=$(grep -c '"status": 101' "$LOG")
sleep 2.5
check "node stopped after 4002 (no further dials)" "[[ \$(grep -c '\"status\": 101' '$LOG') == $n_before ]]"
check "node log records the switchboard stop" "grep -qi 'switchboard: stopped (replaced)' /tmp/winnode.log"

echo "RESULT: $pass passed, $fail failed"
[[ $fail -eq 0 ]]

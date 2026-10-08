#!/usr/bin/env bash
# Windows upstream (browser_*) forwarding against a mock MCP upstream (WSL python, reached
# on 127.0.0.1 via WSL2 loopback). Verifies the node re-serves the upstream's tools, calls
# are forwarded, and a dead upstream does not break local tools. Profile filtering of
# browser_* (desktop 15 / observe none) is platform-independent and covered by unit tests.
set -u
cd "$(dirname "$0")/.."
EXE_SRC=target/x86_64-pc-windows-gnu/release/reverse-attach.exe
WIN_DIR=/mnt/c/Users/$(cmd.exe /c 'echo %USERNAME%' 2>/dev/null | tr -d '\r')/src/imcp-winpoc
MOCKP=18100; PORT=8796
pass=0; fail=0
ok(){ echo "PASS $1"; pass=$((pass+1)); }; bad(){ echo "FAIL $1"; fail=$((fail+1)); }
check(){ if eval "$2"; then ok "$1"; else bad "$1 :: $2"; fi; }
mkdir -p "$WIN_DIR"; cp "$EXE_SRC" "$WIN_DIR/oab-imcp-winpoc.exe"
cmd.exe /c "taskkill /im oab-imcp-winpoc.exe /f" >/dev/null 2>&1
PORT=$MOCKP python3 windows/mock_upstream.py >/tmp/mu.out 2>&1 & MK=$!
trap 'cmd.exe /c "taskkill /im oab-imcp-winpoc.exe /f" >/dev/null 2>&1; kill $MK 2>/dev/null' EXIT
(cd "$WIN_DIR" && ./oab-imcp-winpoc.exe --insecure-local --upstream "browser=http://127.0.0.1:$MOCKP/mcp" >/tmp/winnode.log 2>&1 &)
sleep 1.6
call(){ printf '%s' "$1" > "$WIN_DIR/req.json"; curl.exe -s -m 8 -X POST "http://127.0.0.1:$PORT/mcp" --data-binary @"$(wslpath -w "$WIN_DIR/req.json")"; }
names(){ python3 -c 'import json,sys;print(",".join(t["name"] for t in json.load(sys.stdin)["result"]["tools"]))'; }

echo "== upstream re-served =="
TL=$(call '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | names)
echo "owner tools: $TL"
check "local tools present" "[[ '$TL' == *sys_info* && '$TL' == *powershell* ]]"
check "browser_navigate re-served from the upstream" "[[ '$TL' == *browser_navigate* ]]"
check "browser_snapshot re-served" "[[ '$TL' == *browser_snapshot* ]]"
check "browser_evaluate re-served to owner" "[[ '$TL' == *browser_evaluate* ]]"

echo "== forwarding a call =="
r=$(call '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"browser_navigate","arguments":{"url":"http://example.com"}}}')
check "browser_navigate call forwarded to the upstream" "[[ '$(echo "$r" | python3 -c 'import json,sys;print(json.load(sys.stdin)["result"]["structuredContent"]["ran"])' 2>/dev/null)' == browser_navigate ]]"

echo "== dead upstream does not break local tools =="
kill $MK 2>/dev/null; sleep 0.5
TL2=$(call '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | names)
# browser_* may still be listed briefly (the upstream tool list is cached ~30 s, as on
# macOS/Linux); the resilience requirement is that LOCAL tools keep working and a browser
# call now fails gracefully instead of hanging or breaking the node.
check "local tools still listed with the upstream down" "[[ '$TL2' == *sys_info* && '$TL2' == *powershell* ]]"
r=$(call '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"sys_info","arguments":{}}}')
check "a local tool still works with the upstream down" "[[ '$(echo "$r" | python3 -c 'import json,sys;print(json.load(sys.stdin)["result"]["structuredContent"]["agent"]["platform"])' 2>/dev/null)' == windows ]]"
r=$(call '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"browser_navigate","arguments":{"url":"http://x"}}}')
check "a browser call fails gracefully while the upstream is down" "[[ '$(echo "$r" | python3 -c 'import json,sys;print("error" in json.load(sys.stdin))' 2>/dev/null)' == True ]]"

echo "RESULT: $pass passed, $fail failed"
[[ $fail -eq 0 ]]

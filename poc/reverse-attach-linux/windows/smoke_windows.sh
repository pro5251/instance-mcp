#!/usr/bin/env bash
# Smoke test for the Windows build, driven from WSL on the same machine: the .exe runs
# on Windows (interop) and is called with Windows' own curl.exe on its loopback.
# Needs: a logged-in, unlocked desktop. Takes screenshots of it (kept only in memory).
set -u
cd "$(dirname "$0")/.."
EXE_SRC=target/x86_64-pc-windows-gnu/release/reverse-attach.exe
WIN_DIR=${WIN_DIR:-/mnt/c/Users/$(cmd.exe /c 'echo %USERNAME%' 2>/dev/null | tr -d '\r')/src/imcp-winpoc}
PORT=${PORT:-8796}
pass=0; fail=0
ok()   { echo "PASS $1"; pass=$((pass+1)); }
bad()  { echo "FAIL $1"; fail=$((fail+1)); }
check(){ if eval "$2"; then ok "$1"; else bad "$1 :: $2"; fi; }

mkdir -p "$WIN_DIR" && cp "$EXE_SRC" "$WIN_DIR/oab-imcp-winpoc.exe"
WIN_EXE=$(wslpath -w "$WIN_DIR/oab-imcp-winpoc.exe")
WORK=$(mktemp -d)
stop() { cmd.exe /c "taskkill /im oab-imcp-winpoc.exe /f" >/dev/null 2>&1; }
trap 'stop; rm -rf "$WORK"' EXIT
stop

# rpc <method> <params-json> [profile-path]: POST to /mcp, reply saved to $WORK/last.json
N=0
rpc() {
  N=$((N+1))
  printf '{"jsonrpc":"2.0","id":%d,"method":"%s","params":%s}' "$N" "$1" "${2:-{\}}" > "$WIN_DIR/req.json"
  curl.exe -s -m 20 -X POST "http://127.0.0.1:$PORT/mcp" -H "Content-Type: application/json" \
    --data-binary "@$(wslpath -w "$WIN_DIR/req.json")" > "$WORK/last.json"
}

echo "== start-up =="
(cd "$WIN_DIR" && timeout 5 ./oab-imcp-winpoc.exe > "$WORK/noauth.log" 2>&1); rc=$?
check "refuses to start without auth" "[[ $rc == 2 ]] && grep -q 'refusing to start with no auth' '$WORK/noauth.log'"
(cd "$WIN_DIR" && ./oab-imcp-winpoc.exe --insecure-local > "$WORK/node.log" 2>&1 &)
sleep 1.5
check "listens on the POC port $PORT" "grep -q '127.0.0.1:$PORT' '$WORK/node.log'"

echo "== initialize / tools/list =="
rpc initialize
name=$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["result"]["serverInfo"]["name"])' "$WORK/last.json")
check "serverInfo is the POC name" "[[ '$name' == oab-imcp-winpoc ]]"
python3 -c 'import json,sys;t=json.load(open(sys.argv[1]))["result"]["instructions"];assert "Windows" in t and "locked" in t' "$WORK/last.json"; rc=$?
check "instructions describe Windows" "[[ $rc == 0 ]]"
rpc tools/list
check "tools/list = sys_info,screenshot" "[[ \$(python3 -c 'import json,sys;print(\",\".join(t[\"name\"] for t in json.load(open(sys.argv[1]))[\"result\"][\"tools\"]))' '$WORK/last.json') == sys_info,screenshot ]]"

echo "== sys_info =="
rpc tools/call '{"name":"sys_info","arguments":{}}'
python3 - "$WORK/last.json" > "$WORK/sys.txt" <<'PY'
import json,sys
r=json.load(open(sys.argv[1]))["result"]; s=r["structuredContent"]
print("agent", s["agent"]["name"], s["agent"]["platform"])
print("os", s["os"])
print("desktop", s["permissions"]["input_desktop"], s["permissions"]["screen_recording"])
print("displays", len(s["displays"]), "main_first", s["displays"][0]["main"])
for d in s["displays"]: print("display", d["index"], d["pixels"]["width"], d["pixels"]["height"], d["scale_percent"])
print("text_has_summary", "displays:" in r["content"][0]["text"])
PY
cat "$WORK/sys.txt"
check "agent is windows" "grep -q '^agent oab-imcp-winpoc windows' '$WORK/sys.txt'"
check "os names Windows and its build" "grep -qE '^os Windows 1[01].*build [0-9]+' '$WORK/sys.txt'"
check "input desktop usable" "grep -q '^desktop Default True' '$WORK/sys.txt'"
check "primary display listed first" "grep -q 'main_first True' '$WORK/sys.txt'"
check "readable summary text" "grep -q 'text_has_summary True' '$WORK/sys.txt'"

echo "== screenshot =="
# shot <args-json> -> prints "mime width height display_w display_h" from the actual image bytes
shot() {
  rpc tools/call "{\"name\":\"screenshot\",\"arguments\":$1}"
  python3 - "$WORK/last.json" <<'PY'
import json,sys,base64,struct
r=json.load(open(sys.argv[1]))
if "error" in r: print("error", r["error"]["message"]); sys.exit()
c=r["result"]["content"]; img=[x for x in c if x["type"]=="image"][0]; b=base64.b64decode(img["data"])
if b[:8]==b"\x89PNG\r\n\x1a\n": w,h=struct.unpack(">II",b[16:24])
else:
    i=2
    while i<len(b):
        m=b[i+1]; L=struct.unpack(">H",b[i+2:i+4])[0]
        if m in (0xC0,0xC2): h,w=struct.unpack(">HH",b[i+5:i+9]); break
        i+=2+L
s=r["result"]["structuredContent"]
print(img["mimeType"], w, h, s["points"]["width"], s["points"]["height"], "caption" if any(x["type"]=="text" for x in c) else "nocaption")
PY
}
out=$(shot '{}'); echo "default: $out"; read -r m w h dw dh cap <<< "$out"
check "default is PNG at half the primary display" "[[ $m == image/png && $w == $((dw/2)) && $h == $((dh/2)) ]]"
check "caption text alongside the image (as macOS)" "[[ $cap == caption ]]"
out=$(shot '{"scale":1}'); read -r m w h dw dh cap <<< "$out"
check "scale 1 = physical pixels of the display" "[[ $w == $dw && $h == $dh ]]"
ND=$(python3 -c 'import sys;print(sum(1 for l in open(sys.argv[1]) if l.startswith("display ")))' "$WORK/sys.txt")
for i in $(seq 0 $((ND-1))); do
  out=$(shot "{\"display\":$i,\"scale\":1}"); read -r m w h dw dh cap <<< "$out"
  exp=$(awk -v i=$i '$1=="display" && $2==i {print $3"x"$4}' "$WORK/sys.txt")
  check "display $i at scale 1 is ${exp} (sys_info agrees, DPI-aware)" "[[ ${w}x${h} == $exp ]]"
done
out=$(shot '{"region":{"x":100,"y":50,"width":400,"height":300},"scale":1}'); read -r m w h dw dh cap <<< "$out"
check "region crop is 400x300" "[[ $w == 400 && $h == 300 ]]"
out=$(shot '{"format":"jpeg","quality":0.6}'); read -r m w h dw dh cap <<< "$out"
check "jpeg with 0–1 quality" "[[ $m == image/jpeg ]]"
out=$(shot '{"display":99}'); check "unknown display is an error" "[[ '$out' == error*'out of range'* ]]"
out=$(shot '{"scale":9}');    check "bad scale is an error" "[[ '$out' == error*scale* ]]"

echo "== profiles over the attach plane are unchanged (forced call) =="
rpc tools/call '{"name":"bash","arguments":{"command":"x"}}'
check "unknown tool on Windows" "grep -q 'unknown tool: bash' '$WORK/last.json'"

echo "RESULT: $pass passed, $fail failed"
[[ $fail -eq 0 ]]

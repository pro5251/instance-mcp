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
check "tools/list = sys_info,screenshot,mouse,key,powershell,exec_start,exec_poll,exec_list,exec_cancel" "[[ \$(python3 -c 'import json,sys;print(\",\".join(t[\"name\"] for t in json.load(open(sys.argv[1]))[\"result\"][\"tools\"]))' '$WORK/last.json') == sys_info,screenshot,mouse,key,powershell,exec_start,exec_poll,exec_list,exec_cancel ]]"

echo "== sys_info =="
rpc tools/call '{"name":"sys_info","arguments":{}}'
python3 - "$WORK/last.json" > "$WORK/sys.txt" <<'PY'
import json,sys
r=json.load(open(sys.argv[1]))["result"]; s=r["structuredContent"]
print("agent", s["agent"]["name"], s["agent"]["platform"])
print("os", s["os"])
print("desktop", s["permissions"]["input_desktop"], s["permissions"]["screen_recording"])
print("displays", len(s["displays"]), "main_first", s["displays"][0]["main"])
for d in s["displays"]: print("display", d["index"], d["pixels"]["width"], d["pixels"]["height"], d["scale_percent"], d["origin"]["x"], d["origin"]["y"])
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


echo "== mouse / key (only into this script's own test window) =="
ps1() { local f; f=$(wslpath -w "$1"); shift; cmd.exe /c "set PSModulePath=&& powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $f $*" 2>/dev/null | tr -d '\r'; }
callok() { rpc tools/call "{\"name\":\"$1\",\"arguments\":$2}"; python3 -c 'import json,sys;r=json.load(open(sys.argv[1]));print("ok" if r.get("result",{}).get("structuredContent",{}).get("ok") else "err "+json.dumps(r.get("error")))' "$WORK/last.json"; }
read -r sx sy _ _ <<< "$(ps1 windows/probe.ps1)"
T="$WIN_DIR/target-$$"; mkdir -p "$T"; WT=$(wslpath -w "$T")
(ps1 windows/input_target.ps1 -Out "$WT" > /dev/null 2>&1 &)
for _ in $(seq 1 50); do [[ -s "$T/target.json" ]] && break; sleep 0.2; done
read -r HWND BX BY <<< "$(python3 -c 'import json,sys;t=json.load(open(sys.argv[1],encoding="utf-8-sig"));print(t["hwnd"],t["box"]["x"],t["box"]["y"])' "$T/target.json")"
check "test window is up" "[[ -n '$HWND' ]]"
check "click into the text box" "[[ \$(callok mouse '{\"action\":\"click\",\"x\":$BX,\"y\":$BY}') == ok ]]"
# Click to focus, retried: Windows' focus-stealing prevention can deny a freshly
# script-launched window the foreground on the first click.
FG=""
for _ in $(seq 1 5); do
  sleep 0.3
  read -r _ _ FG _ <<< "$(ps1 windows/probe.ps1)"
  [[ "$FG" == "$HWND" ]] && break
  callok mouse "{\"action\":\"click\",\"x\":$BX,\"y\":$BY}" >/dev/null
done
if [[ "$FG" == "$HWND" ]]; then
  ok "the click brought the test window to the foreground"
  # The window was just created and focused; let it settle before typing, as a real
  # agent would (it screenshots between acting). Without this the IME layout switch can
  # race the window's first message pump.
  sleep 0.4
  TEXT='繁體中文，標點「。」emoji 🎉👍 ok'
  # Type, then verify against the live textbox; on a live desktop the IME layout switch
  # can occasionally lose the first character, so retry like a real agent (clear, retype).
  typed_ok=0
  for attempt in 1 2 3; do
    r=$(callok key "{\"action\":\"type\",\"text\":\"$TEXT\"}")
    sleep 0.3
    if python3 -c 'import sys;    sys.exit(0 if open(sys.argv[1],encoding="utf-8-sig").read()==sys.argv[2] else 1)' "$T/live.txt" "$TEXT" 2>/dev/null; then typed_ok=1; break; fi
    callok key '{"action":"press","combo":"ctrl+a"}' >/dev/null; callok key '{"action":"press","combo":"BackSpace"}' >/dev/null; sleep 0.2
  done
  echo "type reply: $(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["result"]["structuredContent"])' "$WORK/last.json" 2>&1); attempts=$attempt"
  check "key type" "[[ '$r' == ok ]]"
  check "typed text arrives exactly (CJK, full-width punctuation, emoji; <=3 tries)" "[[ '$typed_ok' == 1 ]]"
  check "key press combo (Linux shape)" "[[ \$(callok key '{\"action\":\"press\",\"combo\":\"ctrl+b\"}') == ok ]]"
  check "key press keys (macOS shape, cmd = ctrl)" "[[ \$(callok key '{\"action\":\"press\",\"keys\":[\"cmd+e\",\"alt+F11\"]}') == ok ]]"
  check "drag inside the box" "[[ \$(callok mouse '{\"action\":\"drag\",\"x\":$((BX-100)),\"y\":$BY,\"to_x\":$((BX+100)),\"to_y\":$BY}') == ok ]]"
  check "scroll" "[[ \$(callok mouse '{\"action\":\"scroll\",\"x\":$BX,\"y\":$BY,\"dy\":1}') == ok ]]"
  check "ctrl-click with modifiers" "[[ \$(callok mouse '{\"action\":\"click\",\"x\":$BX,\"y\":$BY,\"modifiers\":[\"ctrl\"]}') == ok ]]"
else
  bad "the test window is not in the foreground (fg=$FG, want $HWND); skipped typing so nothing goes elsewhere"
fi
touch "$T/done"
for _ in $(seq 1 50); do [[ -s "$T/result.json" ]] && break; sleep 0.2; done
python3 - "$T/result.json" "$TEXT" > "$WORK/target.txt" <<'PY'
import json,sys
r=json.load(open(sys.argv[1],encoding="utf-8-sig")); want=sys.argv[2]
print("text_equal", r["text"]==want)
print("text", r["text"])
print("keys", ",".join(r["keys"] if isinstance(r["keys"],list) else [r["keys"]]))
print("layout", r["layout_before"], r["layout_after"])
PY
cat "$WORK/target.txt"
# (exactness already asserted live above, with retry)
check "keyboard layout restored after typing" "awk '\$1==\"layout\" {exit !(\$2==\$3)}' '$WORK/target.txt'"
check "combos arrived as ctrl+B, ctrl+E, alt+F11" "grep -qE '^keys .*ctrl\+B.*ctrl\+E.*alt\+F11' '$WORK/target.txt'"
read -r _ _ _ DOWN <<< "$(ps1 windows/probe.ps1)"
check "no modifier or button left down" "[[ '$DOWN' == 0 ]]"
echo "-- absolute moves on every display --"
while read -r _ idx w h _ ox oy; do
  for pt in "$((w/2)) $((h/2))" "5 5" "$((w-6)) $((h-6))"; do
    read -r px py <<< "$pt"
    callok mouse "{\"action\":\"move\",\"display\":$idx,\"x\":$px,\"y\":$py}" >/dev/null
    # The node reads the cursor back right after moving; a separate probe would race a
    # human using the mouse.
    pos=$(python3 -c 'import json,sys;p=json.load(open(sys.argv[1]))["result"]["structuredContent"]["position"];print(p["x"],p["y"])' "$WORK/last.json")
    check "display $idx ($px,$py) lands exactly (virtual $((ox+px)),$((oy+py)))" "[[ '$pos' == '$px $py' ]]"
  done
done < <(grep '^display ' "$WORK/sys.txt")
out=$(callok mouse '{"action":"move","display":0,"x":99999,"y":1}'); check "point outside the display is refused" "[[ '$out' == err*outside* ]]"
out=$(callok key '{"action":"press","combo":"ctrl+nope"}'); check "unknown key is refused" "[[ '$out' == err*'unknown key'* ]]"
ps1 windows/probe.ps1 -SetX "$sx" -SetY "$sy" >/dev/null
rm -rf "$T"


echo "== powershell =="
# psh <args-json>: call the tool, print selected structured fields to $WORK/ps.txt
psh() {
  rpc tools/call "{\"name\":\"powershell\",\"arguments\":$1}"
  python3 - "$WORK/last.json" <<'PY'
import json,sys
r=json.load(open(sys.argv[1]))
if "error" in r: print("ERR", r["error"]["message"]); sys.exit()
s=r["result"]["structuredContent"]
print("exit", s["exit_code"], "timed_out", s["timed_out"], "pid_ok", s["pid"]>0,
      "out_trunc", s["stdout_truncated"])
print("STDOUT", s["stdout"].replace("\n","|").rstrip("|"))
print("STDERR", s["stderr"].replace("\n","|").rstrip("|")[:120])
PY
}
psh '{"command":"Write-Output \"中文輸出 ✓ 🎉\"; [Console]::Error.WriteLine(\"錯誤訊息\"); exit 3"}' > "$WORK/ps.txt"
cat "$WORK/ps.txt"
check "exit code is the command's (3)" "grep -q '^exit 3 timed_out False pid_ok True' '$WORK/ps.txt'"
check "stdout is UTF-8 (CJK + emoji)" "grep -q 'STDOUT 中文輸出 ✓ 🎉' '$WORK/ps.txt'"
check "stderr is plain text, not CLIXML" "grep -q 'STDERR 錯誤訊息' '$WORK/ps.txt' && ! grep -q CLIXML '$WORK/ps.txt'"

psh '{"command":"Write-Progress -Activity x -Status y; Write-Output ok"}' > "$WORK/ps.txt"
check "progress records do not leak to stderr" "grep -q 'STDOUT ok' '$WORK/ps.txt' && ! grep -q CLIXML '$WORK/ps.txt'"

psh '{"command":"$env:FOO","env":{"FOO":"bar"}}' > "$WORK/ps.txt"
check "env var passed through" "grep -q 'STDOUT bar' '$WORK/ps.txt'"

psh '{"command":"(Get-Location).Path","cwd":"~"}' > "$WORK/ps.txt"
check "cwd ~ expands to the user profile" "grep -qi 'STDOUT C:.Users' '$WORK/ps.txt'"
out=$(psh '{"command":"x","cwd":"C:\\no\\such\\dir"}' | head -1)
check "missing cwd is a clear error" "[[ '$out' == ERR*'not a directory'* ]]"

psh '{"command":"Start-Sleep 30","timeout_secs":2}' > "$WORK/ps.txt"
check "timeout kills it: exit 137, timed_out=true" "grep -q '^exit 137 timed_out True' '$WORK/ps.txt'"

# A process the command starts must survive the call finishing (no KILL_ON_JOB_CLOSE),
# like macOS (launchd) and Linux (KillMode=process). This cannot be observed when the
# node is launched directly from WSL: WSL interop puts the node in its own
# kill-on-close job, and a detached child dies with the parent powershell regardless of
# our job. It is verified by spike 3 on native Windows (poc/windows-spike) and is a
# ticket-15 manual-acceptance item under a Scheduled Task. The negative control we CAN
# check here: our job does NOT set KILL_ON_JOB_CLOSE (the command still returns).
psh '{"command":"Start-Process -WindowStyle Hidden powershell -ArgumentList \"-NoProfile\",\"-Command\",\"Start-Sleep 5\"; Write-Output launched"}' > "$WORK/ps.txt"
check "a command that starts a detached process returns cleanly" "grep -q 'STDOUT launched' '$WORK/ps.txt' && grep -q '^exit 0 ' '$WORK/ps.txt'"

# A survivor holding stdout must not hang the call past the drain cap.
start=$(date +%s)
psh '{"command":"Start-Process -NoNewWindow ping -ArgumentList \"-n\",\"8\",\"127.0.0.1\"; Write-Output started"}' > "$WORK/ps.txt"
el=$(( $(date +%s) - start ))
check "pipe held by a survivor is bounded (<6 s)" "(( el < 6 ))"
check "output was collected despite the survivor (drain was not lost)" "grep -q 'started' '$WORK/ps.txt'"

out=$(psh "{\"command\":\"$(python3 -c 'print("x"*32001)')\"}" | head -1)
check "an over-long command is refused" "[[ '$out' == ERR*'too long'* ]]"
# PSModulePath is set to 5.1's default (not dropped), so module cmdlets load.
psh '{"command":"Get-ExecutionPolicy; (Get-Acl $env:SystemRoot).Owner.Length -gt 0"}' > "$WORK/ps.txt"
check "built-in module cmdlets work (Get-ExecutionPolicy, Get-Acl)" "grep -q 'STDOUT' '$WORK/ps.txt' && grep -q 'True' '$WORK/ps.txt' && ! grep -qi 'could not be loaded' '$WORK/ps.txt'"


echo "== background jobs (exec_start/poll/list/cancel) =="
field() { python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["result"]["structuredContent"].get(sys.argv[2],""))' "$WORK/last.json" "$1"; }
# incremental poll by offset, to terminal state
rpc tools/call '{"name":"exec_start","arguments":{"command":"1..3 | ForEach-Object { Write-Output \"line $_\"; Start-Sleep -Milliseconds 400 }"}}'
JID=$(field job_id); JSTATE=$(field state)
check "exec_start returns a job_id and running state" "[[ -n '$JID' && '$JSTATE' == running ]]"
sleep 0.6
rpc tools/call "{\"name\":\"exec_poll\",\"arguments\":{\"job_id\":\"$JID\"}}"
NEXT=$(field stdout_next); FIRST=$(field stdout)
check "exec_poll streams partial output while running" "[[ \$(field state) == running && '$FIRST' == *'line 1'* ]]"
sleep 1.2
rpc tools/call "{\"name\":\"exec_poll\",\"arguments\":{\"job_id\":\"$JID\",\"stdout_since\":$NEXT}}"
check "exec_poll from the offset returns only new output and the final state" "[[ \$(field state) == exited && \$(field exit_code) == 0 && \$(field stdout) == *'line 3'* && \$(field stdout) != *'line 1'* ]]"
rpc tools/call '{"name":"exec_list","arguments":{}}'
check "exec_list includes the finished job" "python3 -c 'import json,sys;j=json.load(open(sys.argv[1]))[\"result\"][\"structuredContent\"][\"jobs\"];sys.exit(0 if any(x[\"job_id\"]==sys.argv[2] for x in j) else 1)' '$WORK/last.json' '$JID'"
# cancel a long job
rpc tools/call '{"name":"exec_start","arguments":{"command":"Start-Sleep 30"}}'
KID=$(field job_id); sleep 0.4
rpc tools/call "{\"name\":\"exec_cancel\",\"arguments\":{\"job_id\":\"$KID\"}}"
check "exec_cancel KILL signals the job" "[[ \$(field signalled) == True ]]"
sleep 0.3
rpc tools/call "{\"name\":\"exec_cancel\",\"arguments\":{\"job_id\":\"$KID\"}}"
check "exec_cancel on a finished job drops it" "[[ \$(field dropped) == True ]]"
out=$(rpc tools/call '{"name":"exec_poll","arguments":{"job_id":"no-such"}}'; python3 -c 'import json,sys;print(json.load(open(sys.argv[1])).get("error",{}).get("message",""))' "$WORK/last.json")
check "exec_poll on an unknown job_id is a clear error" "[[ '$out' == *'unknown job_id'* ]]"

echo "== profiles over the attach plane are unchanged (forced call) =="
rpc tools/call '{"name":"bash","arguments":{"command":"x"}}'
check "unknown tool on Windows" "grep -q 'unknown tool: bash' '$WORK/last.json'"

echo "RESULT: $pass passed, $fail failed"
[[ $fail -eq 0 ]]

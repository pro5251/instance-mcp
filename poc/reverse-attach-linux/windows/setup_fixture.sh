# Source me (from smoke_install.sh / smoke_setup.sh, cwd = poc/reverse-attach-linux).
# The one test seam of the Windows setup: scripts/setup-winpoc.ps1 with -ApiBase pointed at
# mock_github.py (WSL python, reached on 127.0.0.1 via WSL2 loopback), serving ZIPs packed
# like the release workflow, plus MCP client stubs (claude, codex, copilot, gemini, code,
# code-insiders) first on PATH that log their arguments to stub\<name>.log, and APPDATA
# pointed at a scratch folder so VS Code's user mcp.json is written there.
# Windows processes started from WSL inherit the real Windows PATH and APPDATA (with the
# real clients), so every setup run goes through run-setup.ps1, which swaps them.
WINUSER=$(cmd.exe /c 'echo %USERNAME%' 2>/dev/null | tr -d '\r')
WIN_DIR=/mnt/c/Users/$WINUSER/src/imcp-winpoc-setup
DATA=/mnt/c/Users/$WINUSER/AppData/Local/oab-imcp-winpoc
WORK=$(wslpath "$(cmd.exe /c 'echo %TEMP%' 2>/dev/null | tr -d '\r')")/oab-imcp-winpoc-setup
# real client configs that must never mention the POC server during a smoke
REAL_CONFIGS=(/mnt/c/Users/$WINUSER/.claude.json /mnt/c/Users/$WINUSER/.codex/config.toml
  /mnt/c/Users/$WINUSER/.copilot/mcp-config.json /mnt/c/Users/$WINUSER/.gemini/settings.json
  "/mnt/c/Users/$WINUSER/AppData/Roaming/Code/User/mcp.json"
  "/mnt/c/Users/$WINUSER/AppData/Roaming/Code - Insiders/User/mcp.json")
# real_state: the other configs must stay byte-identical; ~/.claude.json is rewritten by any
# running Claude Code session, so for it only the POC server name is counted.
real_state(){ local f; grep -c oab-imcp-winpoc "${REAL_CONFIGS[0]}" 2>/dev/null; for f in "${REAL_CONFIGS[@]:1}"; do md5sum "$f" 2>/dev/null || echo "absent $f"; done; }
CODEX_VAR=OAB_IMCP_WINPOC_TOKEN
user_env(){ psc "[Environment]::GetEnvironmentVariable('$1','User')"; }
EXE_SRC=${EXE_SRC:-target/x86_64-pc-windows-gnu/release/reverse-attach.exe}
GHPORT=18200; GH="http://127.0.0.1:$GHPORT"
FIX=$(mktemp -d /tmp/oab-winpoc-gh.XXXXXX)
PSMOD="C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\Modules;C:\\Program Files\\WindowsPowerShell\\Modules"
SYS_PATH="C:\\Windows\\System32;C:\\Windows;C:\\Windows\\System32\\WindowsPowerShell\\v1.0;C:\\Windows\\System32\\Wbem"
NODE_DIR=$(powershell.exe -NoProfile -Command '(Get-Command node -CommandType Application -EA 0 | Select -First 1).Source | Split-Path -Parent' 2>/dev/null | tr -d '\r')
pass=0; fail=0
ok(){ echo "PASS $1"; pass=$((pass+1)); }; bad(){ echo "FAIL $1"; fail=$((fail+1)); }
has(){ [[ "$out" == *"$1"* ]]; }
check(){ if eval "$2"; then ok "$1"; else bad "$1 :: $2"; tail -4 <<< "${out:-}" | sed 's/^/     | /'; fi; }
filt(){ tr -d '\r' | grep -vE 'wsl.localhost|UNC paths|CMD.EXE|Defaulting'; }
psc(){ PSModulePath="$PSMOD" powershell.exe -NoProfile -Command "$1" 2>&1 | filt; }

[[ -f "$EXE_SRC" ]] || { echo "missing $EXE_SRC (build for Windows first)"; exit 1; }
mkdir -p "$WIN_DIR/stub"
cp ../../scripts/setup-winpoc.ps1 "$WIN_DIR/"
WIN_DIR_W=$(wslpath -w "$WIN_DIR"); STUB_W="$WIN_DIR_W\\stub"
cat > "$WIN_DIR/run-setup.ps1" <<'EOF'
# smoke helper: run setup-winpoc.ps1 the way setup-winpoc.cmd does, with PATH = $env:SMOKE_PATH
if ($env:SMOKE_PATH) { $env:PATH = $env:SMOKE_PATH }
if ($env:SMOKE_APPDATA) { $env:APPDATA = $env:SMOKE_APPDATA }
& (Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe') -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot 'setup-winpoc.ps1') @args
exit $LASTEXITCODE
EOF
CLIENTS="claude codex copilot gemini code code-insiders"
for c in $CLIENTS; do
  printf '%s\r\n' '@echo off' 'echo %*>>"%~dp0%~n0.log"' \
    'if "%1 %2"=="mcp remove" exit /b 1' \
    'if "%1 %2"=="mcp add" if defined STUB_ADD_RC exit /b %STUB_ADD_RC%' \
    'if "%1 %2"=="mcp get" if defined STUB_GET_RC exit /b %STUB_GET_RC%' \
    'if "%1 %2"=="mcp get" echo %3: http://127.0.0.1:8796/mcp' \
    'if "%1 %2"=="mcp list" echo oab-imcp-winpoc: http://127.0.0.1:8796/mcp' \
    'exit /b 0' > "$WIN_DIR/stub/$c.cmd"
done
clear_logs(){ for c in $CLIENTS; do : > "$WIN_DIR/stub/$c.log"; done; }
clog(){ tr -d '\r' < "$WIN_DIR/stub/$1.log" 2>/dev/null; }   # stubs are .cmd: CRLF lines
clear_logs
# Scratch APPDATA: VS Code Insiders has a user mcp.json with another server (must survive),
# stable VS Code has none yet (must be created).
APPD="$WIN_DIR/appdata"; rm -rf "$APPD"; mkdir -p "$APPD/Code - Insiders/User"
APPD_W=$(wslpath -w "$APPD")
# (non-ASCII text and a one-element array: Windows PowerShell 5.1 must round-trip both)
seed_insiders(){ printf '{\n\t"inputs": [],\n\t"servers": {\n\t\t"mine": { "type": "http", "url": "http://example.invalid/mcp" },\n\t\t"tool": { "type": "stdio", "command": "uvx", "args": ["x"], "description": "我的伺服器" }\n\t}\n}\n' > "$APPD/Code - Insiders/User/mcp.json"; }
seed_insiders; rm -f "$APPD/Code - Insiders/User/"*.bak

# Releases, newest first: a non-winpoc tag (ignored), a draft winpoc-v0.0.3 (skipped), the
# good 0.0.2 and 0.0.1, and an old 0.0.9 whose .sha256 is wrong. test/empty has no winpoc-v*.
python3 - "$FIX" "$EXE_SRC" <<'PY'
import hashlib, json, os, sys, zipfile
fix, exe = sys.argv[1], sys.argv[2]
def pack(ver, bad=False):
    name = f"oab-imcp-winpoc-{ver}-windows-amd64.zip"
    with zipfile.ZipFile(os.path.join(fix, name), "w", zipfile.ZIP_DEFLATED) as z:
        z.write(exe, "oab-imcp-winpoc.exe")
        for f in ("install-winpoc.ps1", "install.cmd", "uninstall.cmd", "pw-mcp.ps1", "README.md"):
            z.write(os.path.join("windows", f), f)
    h = hashlib.sha256(open(os.path.join(fix, name), "rb").read()).hexdigest()
    if bad: h = "0" * 64
    open(os.path.join(fix, name + ".sha256"), "w").write(f"{h}  {name}\n")
    return [{"name": n, "browser_download_url": "{BASE}/dl/" + n} for n in (name, name + ".sha256")]
# published_at decides "newest"; created_at (the tagged commit's date) is set opposite to prove it.
rel = lambda tag, day, assets, draft=False: {"tag_name": tag, "draft": draft, "prerelease": True,
    "created_at": f"2025-{13 - int(day[:2]):02d}-01T00:00:00Z",
    "published_at": None if draft else f"2026-{day}T00:00:00Z", "assets": assets}
json.dump([rel("v9.9.9", "10-09", []),
           rel("winpoc-v0.0.3", "10-08", pack("0.0.3", bad=True), draft=True),
           rel("winpoc-v0.0.2", "10-02", pack("0.0.2")),
           rel("winpoc-v0.0.1", "10-01", pack("0.0.1")),
           rel("winpoc-v0.0.9", "09-01", pack("0.0.9", bad=True))],
          open(os.path.join(fix, "test__winpoc.json"), "w"), indent=1)
json.dump([rel("v9.9.9", "10-09", [])], open(os.path.join(fix, "test__empty.json"), "w"))
PY
cp "$FIX/oab-imcp-winpoc-0.0.1-windows-amd64.zip" "$WIN_DIR/offline.zip"
PORT=$GHPORT ROOT="$FIX" python3 windows/mock_github.py >"$FIX/mock.out" 2>&1 & GHPID=$!
for _ in $(seq 1 50); do grep -q 'mock github on' "$FIX/mock.out" 2>/dev/null && break; sleep 0.1; done

# setup [setup args...]: one run through the seam; output in $out, exit code in $rc.
# SMOKE_PATH picks what is on PATH (default: client stubs + node + system); STUB_GET_RC
# makes the stubs' `mcp get` fail.
setup(){
  local p=${SMOKE_PATH:-"$STUB_W;$NODE_DIR;$SYS_PATH"} wl="SMOKE_PATH:SMOKE_APPDATA"
  [[ -n "${STUB_GET_RC:-}" ]] && wl="$wl:STUB_GET_RC"
  [[ -n "${STUB_ADD_RC:-}" ]] && wl="$wl:STUB_ADD_RC"
  out=$(SMOKE_PATH="$p" SMOKE_APPDATA="$APPD_W" WSLENV="$wl:${WSLENV:-}" \
        PSModulePath="$PSMOD" powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass \
        -File "$WIN_DIR_W\\run-setup.ps1" "$@" 2>&1 | filt; exit "${PIPESTATUS[0]}")
  rc=$?
}
fixture_cleanup(){ kill $GHPID 2>/dev/null; rm -rf "$FIX"; }
task_state(){ psc "(Get-ScheduledTask -TaskName $1 -TaskPath '\\OpenAB-POC\\' -ErrorAction SilentlyContinue).State"; }
task_args(){ psc "(Get-ScheduledTask -TaskName $1 -TaskPath '\\OpenAB-POC\\').Actions[0].Arguments"; }
task_exec(){ psc "(Get-ScheduledTask -TaskName $1 -TaskPath '\\OpenAB-POC\\').Actions[0].Execute"; }

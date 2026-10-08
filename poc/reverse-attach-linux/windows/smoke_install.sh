#!/usr/bin/env bash
# Verifies install-winpoc.ps1 end to end on a real Windows desktop (CurrentUser, no admin):
# install -> healthy -> token auth -> hardened Scheduled Task + token DACL -> uninstall.
# Driven from WSL; the installer runs in Windows PowerShell with a native PSModulePath so
# the ScheduledTasks module loads.
set -u
cd "$(dirname "$0")/.."
EXE_SRC=target/x86_64-pc-windows-gnu/release/reverse-attach.exe
WIN_DIR=/mnt/c/Users/$(cmd.exe /c 'echo %USERNAME%' 2>/dev/null | tr -d '\r')/src/imcp-winpoc
DATA=/mnt/c/Users/$(cmd.exe /c 'echo %USERNAME%' 2>/dev/null | tr -d '\r')/AppData/Local/oab-imcp-winpoc
PORT=8796; TOKEN=poc-smoke-$RANDOM
PS="C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\Modules;C:\\Program Files\\WindowsPowerShell\\Modules"
pass=0; fail=0
ok(){ echo "PASS $1"; pass=$((pass+1)); }; bad(){ echo "FAIL $1"; fail=$((fail+1)); }
check(){ if eval "$2"; then ok "$1"; else bad "$1 :: $2"; fi; }
mkdir -p "$WIN_DIR"; cp "$EXE_SRC" "$WIN_DIR/oab-imcp-winpoc.exe"; cp windows/install-winpoc.ps1 "$WIN_DIR/"
SCRIPT=$(wslpath -w "$WIN_DIR/install-winpoc.ps1")
inst(){ PSModulePath="$PS" powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$SCRIPT" "$@" 2>&1 | tr -d '\r' | grep -vE 'wsl.localhost|UNC paths|CMD.EXE|Defaulting'; }
psc(){ PSModulePath="$PS" powershell.exe -NoProfile -Command "$1" 2>&1 | tr -d '\r' | grep -vE 'wsl.localhost|UNC paths|CMD.EXE|Defaulting'; }
trap 'inst -Uninstall -Purge >/dev/null 2>&1' EXIT
inst -Uninstall -Purge >/dev/null 2>&1

echo "== install =="
out=$(inst -Install -Token "$TOKEN" -Port $PORT -SkipTailscale)
check "installer reports healthy" "[[ '$out' == *'healthy on http'* ]]"
check "the Scheduled Task exists" "[[ '$(psc "(Get-ScheduledTask -TaskName oab-imcp-winpoc -TaskPath '\\OpenAB-POC\\').State")' == Ready || '$(psc "(Get-ScheduledTask -TaskName oab-imcp-winpoc -TaskPath '\\OpenAB-POC\\').State")' == Running ]]"
printf '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' > "$WIN_DIR/req.json"
n=$(curl.exe -s -m 5 -X POST "http://127.0.0.1:$PORT/mcp" -H "Authorization: Bearer $TOKEN" --data-binary @"$(wslpath -w "$WIN_DIR/req.json")" | python3 -c 'import json,sys;print(len(json.load(sys.stdin)["result"]["tools"]))' 2>/dev/null)
check "serves 9 tools with the bearer token" "[[ '$n' == 9 ]]"
check "a wrong token is rejected (401)" "[[ '$(curl.exe -s -o /dev/null -w '%{http_code}' -X POST "http://127.0.0.1:$PORT/mcp" -H 'Authorization: Bearer WRONG' --data-binary @"$(wslpath -w "$WIN_DIR/req.json")")' == 401 ]]"

echo "== hardened task settings =="
set=$(psc "\$s=(Get-ScheduledTask -TaskName oab-imcp-winpoc -TaskPath '\\OpenAB-POC\\').Settings; \"\$(\$s.ExecutionTimeLimit)|\$(\$s.DisallowStartIfOnBatteries)|\$(\$s.MultipleInstances)\"")
echo "settings: $set"
check "no execution time limit (PT0S)" "[[ '$set' == PT0S* ]]"
check "runs on battery" "[[ '$set' == *'|False|'* ]]"
check "ignores a second instance" "[[ '$set' == *'|IgnoreNew' ]]"

echo "== token DACL =="
acl=$(icacls.exe "$(wslpath -w "$DATA/token")" 2>/dev/null | tr -d '\r')
check "token DACL grants the owner and SYSTEM" "grep -qi 'SYSTEM:' <<< '$acl' && grep -qiE '\\\\[a-z0-9_.-]+:' <<< '$acl'"
check "token DACL excludes Users/Everyone" "! grep -qiE 'BUILTIN..Users:|Everyone:|Authenticated Users:' <<< '$acl'"

echo "== uninstall =="
out=$(inst -Uninstall -Purge)
check "uninstall removes program + data" "[[ '$out' == *'removed program + data'* && ! -d '$DATA' ]]"
check "the Scheduled Task is gone" "[[ '$(psc "(Get-ScheduledTask -TaskName oab-imcp-winpoc -TaskPath '\\OpenAB-POC\\' -ErrorAction SilentlyContinue) -eq \$null")' == True ]]"

echo "RESULT: $pass passed, $fail failed"
[[ $fail -eq 0 ]]

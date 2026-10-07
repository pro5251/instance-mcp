#!/usr/bin/env bash
# Smoke test: the macOS-compatible command-line flags (instance-mcp#32 item 8).
# Each node is started with flags only, from a clean MCP_* environment, except where
# a case checks that a flag wins over its environment variable.
set -u
cd "$(dirname "$0")"
BIN=./target/release/reverse-attach
pass=0; fail=0
ok()   { echo "PASS $1"; pass=$((pass+1)); }
bad()  { echo "FAIL $1"; fail=$((fail+1)); }
check(){ if eval "$2"; then ok "$1"; else bad "$1 :: $2"; fi; }
clean() { env -u BIND -u MCP_TOKEN -u MCP_TOKEN_FILE -u MCP_ALLOW_LOGIN -u MCP_INSECURE_LOCAL -u MCP_UPSTREAM -u MCP_GRANTS_FILE "$@"; }
WORK=$(mktemp -d); PIDS=()
trap 'kill "${PIDS[@]}" 2>/dev/null; rm -rf "$WORK"' EXIT
LIST='{"jsonrpc":"2.0","id":1,"method":"tools/list"}'
C="curl -s -m 5"
code() { $C -o /dev/null -w '%{http_code}' "$@"; }

echo "== one-shot flags =="
v=$(clean $BIN --version); check "--version prints the crate version" "[[ '$v' =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]"
clean $BIN --bogus >/dev/null 2>"$WORK/bogus.err"; rc=$?
check "unknown flag exits 64" "[[ $rc == 64 ]]"
check "unknown flag names itself" "grep -q 'unknown flag --bogus' '$WORK/bogus.err'"
clean $BIN >/dev/null 2>"$WORK/noauth.err"; rc=$?
check "no flags and no env still refuses to start without auth" "[[ $rc == 2 ]] && grep -q 'refusing to start with no auth' '$WORK/noauth.err'"

echo "== --host/--port/--path/--no-attach/--insecure-local =="
clean $BIN --host 127.0.0.1 --port 8797 --path /x --no-attach --insecure-local --no-grant-persistence > "$WORK/a.log" 2>&1 & PIDS+=($!)
sleep 0.4
n=$($C -X POST 127.0.0.1:8797/x -d "$LIST" | python3 -c 'import sys,json;print(len(json.load(sys.stdin)["result"]["tools"]))' 2>/dev/null)
check "MCP served on --path" "[[ '$n' == 5 ]]"
check "default /mcp is not served when --path moves it" "[[ $(code -X POST 127.0.0.1:8797/mcp -d "$LIST") == 404 ]]"
check "--no-attach: POST /attach is 404" "[[ $(code -X POST 127.0.0.1:8797/attach -d '{}') == 404 ]]"
check "--no-attach: GET /attach/x is 404" "[[ $(code 127.0.0.1:8797/attach/x) == 404 ]]"
check "--no-grant-persistence reported" "grep -q 'persistence off (MCP_GRANTS_FILE=off)' '$WORK/a.log'"

echo "== --token-file, and a flag wins over its variable =="
printf 'cli-token\n' > "$WORK/token"
clean env MCP_TOKEN=env-token $BIN --port 8798 --token-file "$WORK/token" --no-grant-persistence > "$WORK/b.log" 2>&1 & PIDS+=($!)
sleep 0.4
check "no bearer → 401" "[[ $(code -X POST 127.0.0.1:8798/mcp -d "$LIST") == 401 ]]"
check "the environment's token no longer works" "[[ $(code -X POST 127.0.0.1:8798/mcp -H 'Authorization: Bearer env-token' -d "$LIST") == 401 ]]"
check "the --token-file token works" "[[ $(code -X POST 127.0.0.1:8798/mcp -H 'Authorization: Bearer cli-token' -d "$LIST") == 200 ]]"
check "token never logged" "! grep -q cli-token '$WORK/b.log'"

echo "== --allow-login (repeatable) =="
clean $BIN --port 8799 --allow-login a@x.com --allow-login b@x.com --no-grant-persistence > "$WORK/c.log" 2>&1 & PIDS+=($!)
sleep 0.4
check "listed login accepted" "[[ $(code -X POST 127.0.0.1:8799/mcp -H 'Tailscale-User-Login: b@x.com' -d "$LIST") == 200 ]]"
check "other login refused" "[[ $(code -X POST 127.0.0.1:8799/mcp -H 'Tailscale-User-Login: c@x.com' -d "$LIST") == 401 ]]"

echo "RESULT: $pass passed, $fail failed"
[[ $fail -eq 0 ]]

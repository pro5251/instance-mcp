#!/usr/bin/env bash
# Smoke test: reverse attach over TLS (https:// mint, wss:// dial, prompt DELETE)
# against mock_runtime.py with a throwaway self-signed certificate. The node
# trusts it only through SSL_CERT_FILE, so the "untrusted" case proves that the
# certificate is verified and nothing falls back to plain HTTP.
set -u
cd "$(dirname "$0")"
BIN=./target/release/reverse-attach
pass=0; fail=0
ok()   { echo "PASS $1"; pass=$((pass+1)); }
bad()  { echo "FAIL $1"; fail=$((fail+1)); }
check(){ if eval "$2"; then ok "$1"; else bad "$1 :: $2"; fi; }

WORK=$(mktemp -d)
# A throwaway CA and a localhost leaf it signs (webpki refuses a CA used as the server certificate).
openssl req -x509 -newkey rsa:2048 -nodes -days 1 -subj "/CN=smoke-tls test CA" \
  -keyout "$WORK/ca.key" -out "$WORK/ca.pem" 2>/dev/null
openssl req -newkey rsa:2048 -nodes -subj /CN=localhost -keyout "$WORK/key.pem" -out "$WORK/leaf.csr" 2>/dev/null
printf 'subjectAltName=DNS:localhost\nbasicConstraints=CA:FALSE\nextendedKeyUsage=serverAuth\n' > "$WORK/leaf.ext"
openssl x509 -req -in "$WORK/leaf.csr" -CA "$WORK/ca.pem" -CAkey "$WORK/ca.key" -CAcreateserial \
  -days 1 -extfile "$WORK/leaf.ext" -out "$WORK/cert.pem" 2>/dev/null
export MCP_GRANTS_FILE="$WORK/grants.json"

PORT=18093 ADMIN=admin-secret CLOSES=1000 SECRETS=pre=preminted-xyz LOG="$WORK/mock.jsonl" \
  TLS_CERT="$WORK/cert.pem" TLS_KEY="$WORK/key.pem" python3 mock_runtime.py > "$WORK/mock.out" 2>&1 &
MOCK=$!
# Trusted node: the throwaway CA is its only extra root.
SSL_CERT_FILE="$WORK/ca.pem" BIND=127.0.0.1:8791 MCP_INSECURE_LOCAL=1 $BIN > "$WORK/trusted.log" 2>&1 &
RA=$!
# Untrusted node: system roots only.
BIND=127.0.0.1:8792 MCP_INSECURE_LOCAL=1 MCP_GRANTS_FILE=off $BIN > "$WORK/untrusted.log" 2>&1 &
RB=$!
trap 'kill $MOCK $RA $RB 2>/dev/null; rm -rf "$WORK"' EXIT
sleep 0.6
C="curl -s -m 8"
field() { python3 -c 'import sys,json;d=json.load(sys.stdin);print(d.get(sys.argv[1],""))' "$1"; }
state_of() { $C 127.0.0.1:8791/attach/"$1" | field state; }

echo "== https mint + wss dial =="
raw=$($C -w '\n%{http_code}' -X POST 127.0.0.1:8791/attach -d '{"runtime":"wss://localhost:18093","session":"tls","profile":"owner","ttl_secs":60,"admin_credential":"admin-secret"}')
code=$(echo "$raw" | tail -1); r=$(echo "$raw" | sed '$d'); echo "$r"
check "https mint → 202" "[[ '$code' == 202 ]]"
GID=$(echo "$r" | field id)
for _ in $(seq 1 50); do grep -q '"ev": "closed"' "$WORK/mock.jsonl" 2>/dev/null && break; sleep 0.2; done
check "mock saw the mint with the admin credential" "grep -q '\"ev\": \"mint\"' '$WORK/mock.jsonl'"
check "wss attach upgraded (101)" "grep -q '\"status\": 101' '$WORK/mock.jsonl'"
check "MCP answered over wss" "grep -q 'tools/list' '$WORK/mock.jsonl'"
check "admin credential never logged by the node" "! grep -q admin-secret '$WORK/trusted.log'"

echo "== pre-minted secret over wss, then DELETE within ~1 s =="
raw=$($C -w '\n%{http_code}' -X POST 127.0.0.1:8791/attach -d '{"runtime":"wss://localhost:18093","session":"pre","profile":"desktop","ttl_secs":60,"secret":"preminted-xyz"}')
PID_=$(echo "$raw" | sed '$d' | field id)
for _ in $(seq 1 30); do [[ "$(state_of "$PID_")" == attached ]] && break; sleep 0.2; done
check "pre-minted wss grant attached" "[[ '$(state_of "$PID_")' == attached ]]"
t0=$(date +%s%N)
$C -X DELETE 127.0.0.1:8791/attach/"$PID_" >/dev/null
for _ in $(seq 1 30); do [[ "$(state_of "$PID_")" != attached ]] && break; sleep 0.1; done
ms=$(( ($(date +%s%N) - t0) / 1000000 ))
echo "DELETE took effect in ${ms} ms"
check "DELETE ends a wss attachment within 1.5 s" "(( ms <= 1500 ))"

echo "== untrusted certificate: refused, never downgraded =="
mints_before=$(grep -c '"ev": "mint"' "$WORK/mock.jsonl")
r=$($C -X POST 127.0.0.1:8792/attach -d '{"runtime":"wss://localhost:18093","session":"tls","profile":"owner","ttl_secs":60,"admin_credential":"admin-secret"}')
echo "$r"
check "untrusted https mint fails" "[[ '$r' == *'mint failed'* ]]"
check "failure names the certificate" "[[ '$r' == *ertificate* || '$r' == *UnknownIssuer* ]]"
sleep 0.3
check "the runtime saw a refused TLS handshake" "grep -q 'tls_handshake_failed' '$WORK/mock.jsonl'"
check "no mint request reached the runtime (no plain-HTTP fallback)" "[[ \$(grep -c '\"ev\": \"mint\"' '$WORK/mock.jsonl') == $mints_before ]]"

echo "RESULT: $pass passed, $fail failed"
[[ $fail -eq 0 ]]

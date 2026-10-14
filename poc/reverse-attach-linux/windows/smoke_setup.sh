#!/usr/bin/env bash
# Verifies the setup bootstrap's release handling through its -ApiBase seam (fake GitHub):
# newest winpoc-v* (skipping drafts and other tags), -Version, sha256 mismatch, no release,
# API failures, -ZipPath offline, and the installer's exit code passed through. Installs use
# -NoStart -NoBrowser -NoClaudeCode so each run is quick; smoke_install.sh covers the node.
set -u
cd "$(dirname "$0")/.."
. windows/setup_fixture.sh
API="-ApiBase $GH -Repo test/winpoc"
FAST="-NoStart -NoBrowser -NoClaudeCode -SkipTailscale -Token poc-smoke-$RANDOM"
uninst(){ setup -ZipPath "$(wslpath -w "$WIN_DIR/offline.zip")" -Uninstall -Purge >/dev/null; }
trap 'uninst; fixture_cleanup' EXIT
uninst

echo "== newest release =="
: > "$FIX/requests.log"
setup $API $FAST
check "setup exits 0" "[[ $rc == 0 ]]"
check "picks winpoc-v0.0.2 (skips the draft 0.0.3 and the v9.9.9 tag)" "has 'release winpoc-v0.0.2 from test/winpoc'"
check "downloads the 0.0.2 ZIP and its .sha256" "grep -q 'dl/oab-imcp-winpoc-0.0.2-windows-amd64.zip$' '$FIX/requests.log' && grep -q '0.0.2-windows-amd64.zip.sha256$' '$FIX/requests.log'"
check "never fetches the draft's assets" "! grep -q '0.0.3' '$FIX/requests.log'"
check "verifies the sha256" "has 'sha256 ok'"
check "runs the installer from the ZIP (task registered)" "[[ '$(task_state oab-imcp-winpoc)' == Ready ]]"
check "installer copies itself for offline -Uninstall" "[[ -f /mnt/c/Users/$WINUSER/AppData/Local/Programs/OpenAB-POC/imcp-winpoc/install-winpoc.ps1 ]]"

echo "== -Version =="
setup $API $FAST -Version 0.0.1
check "-Version 0.0.1 installs winpoc-v0.0.1" "[[ $rc == 0 ]] && has 'release winpoc-v0.0.1'"
setup $API $FAST -Version winpoc-v0.0.2
check "-Version accepts the full tag" "[[ $rc == 0 ]] && has 'release winpoc-v0.0.2'"
setup $API $FAST -Version winpoc-v0.0.3
check "a draft cannot be picked by -Version" "[[ $rc != 0 ]] && has 'release winpoc-v0.0.3 not found'"
check "the not-found message lists what exists" "has 'winpoc-v0.0.2, winpoc-v0.0.1, winpoc-v0.0.9'"

echo "== sha256 mismatch =="
uninst
setup $API $FAST -Version 0.0.9
check "mismatch exits non-zero" "[[ $rc != 0 ]]"
check "mismatch says so and that nothing was installed" "has 'sha256 mismatch' && has 'installed nothing'"
check "the download is deleted" "[[ ! -e '$WORK/oab-imcp-winpoc-0.0.9-windows-amd64.zip' ]]"
check "the installer did not run (no task)" "[[ -z '$(task_state oab-imcp-winpoc)' ]]"

echo "== no release / API failures =="
setup -ApiBase $GH -Repo test/empty $FAST
check "no winpoc-v* release: clear error, non-zero" "[[ $rc != 0 ]] && has 'no winpoc-v* release found in test/empty'"
check "no release: suggests -ZipPath" "has 'next:' && has 'setup-winpoc.cmd -ZipPath'"
setup -ApiBase $GH -Repo test/nosuch $FAST
check "unknown repo: says 404, non-zero" "[[ $rc != 0 ]] && has 'HTTP 404'"
setup -ApiBase $GH -Repo ratelimited/x $FAST
check "rate limited: says so and suggests -ZipPath" "[[ $rc != 0 ]] && has 'rate limit' && has '-ZipPath'"
setup -ApiBase http://127.0.0.1:1 -Repo test/winpoc $FAST
check "API unreachable: clear error, non-zero" "[[ $rc != 0 ]] && has 'cannot list releases'"
check "no PowerShell stack trace in the failures" "! has 'CategoryInfo' && ! has 'FullyQualifiedErrorId'"

echo "== -ZipPath offline =="
setup -ApiBase http://127.0.0.1:1 -ZipPath "$(wslpath -w "$WIN_DIR/offline.zip")" $FAST
check "-ZipPath installs with no API reachable" "[[ $rc == 0 ]] && has 'local ZIP' && [[ '$(task_state oab-imcp-winpoc)' == Ready ]]"
setup -ZipPath 'C:\nonexistent\x.zip' $FAST
check "-ZipPath to a missing file fails clearly" "[[ $rc != 0 ]] && has 'no such ZIP'"

echo "== exit code passthrough =="
setup $API -Install -Token x -AllowLogin a@b.c -NoStart -Port notanumber
check "an installer failure is returned as a non-zero exit" "[[ $rc != 0 ]]"
setup $API -Status
check "-Status is forwarded (no -Install added)" "[[ $rc == 0 ]] && has 'task oab-imcp-winpoc :'"

echo "RESULT: $pass passed, $fail failed"
[[ $fail -eq 0 ]]

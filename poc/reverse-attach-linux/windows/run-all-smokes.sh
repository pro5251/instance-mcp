#!/usr/bin/env bash
# Run every Windows POC smoke in sequence (from WSL; builds for Windows first).
# Prereq: `. windows/xenv.sh` toolchain (cargo zigbuild), a logged-in unlocked desktop.
set -u
cd "$(dirname "$0")/.."
. windows/xenv.sh
echo "building for Windows…"; cargo zigbuild --release --target x86_64-pc-windows-gnu 2>&1 | grep -E '^error|Finished' | tail -1
total_fail=0
for s in smoke_windows smoke_reverse smoke_upstream smoke_switchboard smoke_install; do
  echo; echo "========== $s =========="
  bash "windows/$s.sh"; rc=$?
  [[ $rc -ne 0 ]] && total_fail=$((total_fail+1))
done
echo; echo "===================================="
[[ $total_fail -eq 0 ]] && echo "ALL WINDOWS SMOKES PASSED" || echo "$total_fail smoke suite(s) had failures (the CJK-IME typing case is ~1-in-6 flaky on a live desktop; re-run)"

# Playwright MCP beside the Windows hands node (POC). Loopback only; the daemon re-serves
# it via --upstream (never exposed directly). Mirrors pw-mcp.sh for Linux.
# Requires Node.js on PATH and @playwright/mcp installed under this folder (install-winpoc.ps1
# does `npm install @playwright/mcp` and `npx playwright install chromium`).
$ErrorActionPreference = 'Stop'
$Base = Join-Path $env:LOCALAPPDATA 'oab-imcp-winpoc'
$PwDir = Join-Path $Base 'pw-mcp'
$cli = Join-Path $PwDir 'node_modules\.bin\playwright-mcp.cmd'
if (-not (Test-Path $cli)) { Write-Error "playwright-mcp not installed under $PwDir; run install-winpoc.ps1"; exit 1 }
& $cli `
  --host 127.0.0.1 --port 8797 `
  --allowed-hosts "127.0.0.1,localhost,127.0.0.1:8797,localhost:8797" `
  --browser chromium `
  --user-data-dir (Join-Path $Base 'pw-profile') `
  --output-dir (Join-Path $Base 'pw-output') `
  --idle-timeout 1800000 `
  --shared-browser-context --caps vision,pdf

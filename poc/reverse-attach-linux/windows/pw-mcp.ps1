# Playwright MCP beside the Windows hands node (POC). Loopback only; the node re-serves it
# via --upstream (never exposed directly). Mirrors pw-mcp.sh for Linux.
# install-winpoc.ps1 copies this to %LOCALAPPDATA%\Programs\OpenAB-POC\imcp-winpoc\, installs
# @playwright/mcp (pinned) under %LOCALAPPDATA%\oab-imcp-winpoc\pw-mcp with
# `npm install --save-exact` + `playwright install chromium`, and runs this from the logon
# task \OpenAB-POC\oab-imcp-winpoc-pw. -NodeDir is the folder node.exe was found in at install
# time (a Scheduled Task's PATH may lack it, e.g. with nvm/Volta).
param(
  [string]$NodeDir,
  [int]$Port = 8797,                                              # passed by the installer
  [string]$Base = (Join-Path $env:LOCALAPPDATA 'oab-imcp-winpoc')  # passed by the installer
)
$ErrorActionPreference = 'Stop'
$PwDir = Join-Path $Base 'pw-mcp'
$cli = Join-Path $PwDir 'node_modules\.bin\playwright-mcp.cmd'
if ($NodeDir -and (Test-Path $NodeDir)) { $env:PATH = "$NodeDir;$env:PATH" }
if (-not (Test-Path $cli)) { Write-Error "playwright-mcp not installed under $PwDir; re-run setup (scripts\setup-winpoc.cmd)"; exit 1 }
& $cli `
  --host 127.0.0.1 --port $Port `
  --allowed-hosts "127.0.0.1,localhost,127.0.0.1:$Port,localhost:$Port" `
  --browser chromium `
  --user-data-dir (Join-Path $Base 'pw-profile') `
  --output-dir (Join-Path $Base 'pw-output') `
  --idle-timeout 1800000 `
  --shared-browser-context --caps vision,pdf
exit $LASTEXITCODE

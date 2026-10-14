<#  setup-winpoc.ps1 - one-step setup of the Windows POC hands node (oab-imcp-winpoc).
    CurrentUser, no admin. Run from a clone (or double-click setup-winpoc.cmd beside it).

    This script only finds a release, downloads it, verifies it and unpacks it; then it runs
    install-winpoc.ps1 FROM THE ZIP (so the installer and the exe are the same version) and
    returns its exit code. Every argument it does not know is passed to that installer.

    Usage:
      setup-winpoc.ps1 [-Version <tag>] [-ZipPath <zip>] [-Repo <owner/name>] [-ApiBase <url>]
                       [installer args: -Token <str> | -AllowLogin <email>, -NoBrowser,
                        -NoClaudeCode, -Uninstall [-Purge], -Status, ...]

      -Version  install this release (winpoc-v0.0.1 or 0.0.1) instead of the newest winpoc-v*
      -ZipPath  install from a local ZIP: no download, no hash check (offline / rate limited)
      -Repo     GitHub repo with the winpoc-v* releases (default below)
      -ApiBase  GitHub API base URL (tests point it at a local fake)

    Re-running it updates the node; the token is kept. Names are the POC names of spec 13.2.
#>
[CmdletBinding(PositionalBinding=$false)]
param(
  [string]$Version,
  [string]$ZipPath,
  [string]$Repo,
  [string]$ApiBase = 'https://api.github.com',
  [Parameter(ValueFromRemainingArguments=$true)] [string[]]$InstallerArgs = @()
)
# ---- names (POC; change here once the author settles them) ------------------------------
$DefaultRepo = 'pro5251/instance-mcp'   # upstream PR: openabdev/instance-mcp
$TagPrefix   = 'winpoc-v'
$AssetStem   = 'oab-imcp-winpoc'        # oab-imcp-winpoc-<ver>-windows-amd64.zip (+ .sha256)
$WorkDir     = Join-Path $env:TEMP "$AssetStem-setup"
$UserAgent   = "$AssetStem-setup"
# ------------------------------------------------------------------------------------------
$ErrorActionPreference = 'Stop'
if ($PSVersionTable.PSVersion.Major -le 5) {
  # Started from a PowerShell 7 prompt, Windows PowerShell inherits pwsh's module paths and
  # loads its incompatible modules (Get-FileHash, ScheduledTasks go missing). Drop them; the
  # installer inherits the cleaned value.
  $env:PSModulePath = (($env:PSModulePath -split ';') | Where-Object { $_ -and $_ -notmatch '\\PowerShell\\(7[^\\]*\\)?Modules\\?$' }) -join ';'
}
$ProgressPreference = 'SilentlyContinue'   # Windows PowerShell's progress bar makes downloads crawl
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
if (-not $Repo) { $Repo = $DefaultRepo }
$ApiBase = $ApiBase.TrimEnd('/')

function Fail([int]$code, [string]$msg, [string[]]$next = @()) {
  Write-Host ""
  Write-Host "setup failed: $msg" -ForegroundColor Red
  foreach ($n in $next) { Write-Host "  next: $n" }
  exit $code
}

function Get-Release {
  $url = "$ApiBase/repos/$Repo/releases?per_page=100"
  try {
    $list = Invoke-RestMethod -UseBasicParsing -TimeoutSec 30 -Headers @{ 'User-Agent' = $UserAgent; 'Accept' = 'application/vnd.github+json' } $url
  } catch {
    $why = $_.Exception.Message
    $status = $null
    try { $status = [int]$_.Exception.Response.StatusCode } catch {}
    if ($status -eq 403 -or $status -eq 429) { $why = "HTTP $status (GitHub API rate limit?)" }
    elseif ($status -eq 404) { $why = "HTTP 404 (no such repo: $Repo)" }
    Fail 4 "cannot list releases at $url : $why" @(
      "check the network, or wait an hour if GitHub rate limited you",
      "or download $AssetStem-<ver>-windows-amd64.zip from https://github.com/$Repo/releases and run: setup-winpoc.cmd -ZipPath <zip>")
  }
  $cands = @($list | Where-Object { $_.tag_name -like "$TagPrefix*" -and -not $_.draft })
  if ($Version) {
    $want = if ($Version -like "$TagPrefix*") { $Version } else { "$TagPrefix$Version" }
    $rel = $cands | Where-Object { $_.tag_name -eq $want } | Select-Object -First 1
    if (-not $rel) {
      $have = ($cands | ForEach-Object { $_.tag_name }) -join ', '
      if (-not $have) { $have = 'none' }
      Fail 2 "release $want not found in $Repo" @("published $TagPrefix* releases: $have", "or install a local ZIP: setup-winpoc.cmd -ZipPath <zip>")
    }
    return $rel
  }
  # Newest by publish time (created_at is the tagged commit's date); drafts have none.
  $rel = $cands | Sort-Object { if ($_.published_at) { [datetime]$_.published_at } else { [datetime]$_.created_at } } -Descending | Select-Object -First 1
  if (-not $rel) {
    Fail 2 "no $TagPrefix* release found in $Repo" @(
      "the maintainer publishes one by pushing a $TagPrefix* tag (.github/workflows/winpoc-release.yml)",
      "or pass -Repo <owner/name>, or install a local ZIP: setup-winpoc.cmd -ZipPath <zip>")
  }
  return $rel
}

function Get-Asset($rel, [string]$name) {
  $a = @($rel.assets) | Where-Object { $_.name -eq $name } | Select-Object -First 1
  if (-not $a) { Fail 2 "release $($rel.tag_name) has no asset $name" @("pick another release with -Version, or use -ZipPath <zip>") }
  return $a.browser_download_url
}

function Save-Url([string]$url, [string]$out) {
  try { Invoke-WebRequest -UseBasicParsing -TimeoutSec 600 -Headers @{ 'User-Agent' = $UserAgent } -OutFile $out $url }
  catch { Remove-Item -Force $out -ErrorAction SilentlyContinue; Fail 4 "download failed: $url : $($_.Exception.Message)" @("retry, or download it by hand and run: setup-winpoc.cmd -ZipPath <zip>") }
}

# ---- 1. find + 2. download + 3. verify (or take -ZipPath as is) ----------------------------
Remove-Item -Recurse -Force $WorkDir -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $WorkDir | Out-Null
if ($ZipPath) {
  if (-not (Test-Path -LiteralPath $ZipPath -PathType Leaf)) { Fail 2 "no such ZIP: $ZipPath" }
  $zip = Join-Path $WorkDir (Split-Path -Leaf $ZipPath)
  Copy-Item -LiteralPath $ZipPath $zip -Force
  Write-Host "setup: installing from local ZIP $ZipPath (no download, no hash check)"
} else {
  $rel = Get-Release
  $ver = $rel.tag_name.Substring($TagPrefix.Length)
  $zipName = "$AssetStem-$ver-windows-amd64.zip"
  $zipUrl = Get-Asset $rel $zipName
  $shaUrl = Get-Asset $rel "$zipName.sha256"
  Write-Host "setup: release $($rel.tag_name) from $Repo"
  $zip = Join-Path $WorkDir $zipName
  $sha = "$zip.sha256"
  Save-Url $shaUrl $sha
  Save-Url $zipUrl $zip
  $want = ((Get-Content -Raw -LiteralPath $sha).Trim() -split '\s+')[0].ToLower()
  $got = (Get-FileHash -Algorithm SHA256 -LiteralPath $zip).Hash.ToLower()
  if ($want -ne $got) {
    Remove-Item -Recurse -Force $WorkDir -ErrorAction SilentlyContinue
    Fail 3 "sha256 mismatch for $zipName (expected $want, got $got); deleted the download, installed nothing" @(
      "retry (a truncated download also fails this check)",
      "if it fails again, do not install that file; report it on https://github.com/$Repo/issues")
  }
  Write-Host "setup: sha256 ok ($got)"
}

# ---- 4. unpack ------------------------------------------------------------------------------
Unblock-File -LiteralPath $zip   # drop the Mark of the Web so SmartScreen sees a local file
$pkg = Join-Path $WorkDir 'pkg'
try { Expand-Archive -LiteralPath $zip -DestinationPath $pkg -Force }
catch { Fail 2 "cannot unpack $zip : $($_.Exception.Message)" }
Get-ChildItem -Recurse -File $pkg | Unblock-File
$installer = Get-ChildItem -Recurse -File -Filter 'install-winpoc.ps1' $pkg | Select-Object -First 1
if (-not $installer) { Fail 2 "the ZIP has no install-winpoc.ps1" }

# ---- hand over: everything else goes to the installer from the ZIP -------------------------
$fwd = @($InstallerArgs)
if (-not ($fwd | Where-Object { $_ -match '^-(Install|Uninstall|Status)(:.*)?$' })) { $fwd = @('-Install') + $fwd }
$ps = (Get-Process -Id $PID).Path
& $ps -NoProfile -ExecutionPolicy Bypass -File $installer.FullName @fwd
exit $LASTEXITCODE

<#  install-winpoc.ps1 - install/uninstall the Windows POC hands node (oab-imcp-winpoc).
    CurrentUser scope, no admin. Mirrors install-linux.sh / install-prebuilt.sh.
    Names are the POC names of spec section 13.2, pending the author's confirmation.

    Usage:
      install-winpoc.ps1 -Install [-Token <str> | -AllowLogin <email>] [-Port 8796] [-ServePort 8445]
      install-winpoc.ps1 -Uninstall [-Purge]
      install-winpoc.ps1 -Status
#>
[CmdletBinding(DefaultParameterSetName='Status')]
param(
  [Parameter(ParameterSetName='Install')] [switch]$Install,
  [Parameter(ParameterSetName='Uninstall')] [switch]$Uninstall,
  [Parameter(ParameterSetName='Status')] [switch]$Status,
  [string]$Token,
  [string]$AllowLogin,
  [int]$Port = 8796,
  [int]$ServePort = 8445,
  [switch]$Purge,
  [switch]$SkipTailscale,
  [switch]$NoStart          # for CI/tests: create the task but do not start/serve
)
$ErrorActionPreference = 'Stop'
$Name    = 'oab-imcp-winpoc'
$TaskPath= '\OpenAB-POC\'
$TaskName= "$TaskPath$Name"
$ProgDir = Join-Path $env:LOCALAPPDATA 'Programs\OpenAB-POC\imcp-winpoc'
$DataDir = Join-Path $env:LOCALAPPDATA 'oab-imcp-winpoc'
$Exe     = Join-Path $ProgDir "$Name.exe"
$TokenF  = Join-Path $DataDir 'token'
$LogDir  = Join-Path $DataDir 'logs'

function Lock-Down([string]$path) {
  # Protected DACL: current user + SYSTEM only, inheritance disabled. Use icacls with the
  # *SID form (the account-name form silently grants nothing in some shells, leaving an
  # empty DACL that locks the owner out). Directories get container/object inherit so new
  # files under them are born locked too. The owner keeps FullControl, so -Purge can still
  # delete the tree.
  $sid = [System.Security.Principal.WindowsIdentity]::GetCurrent().User.Value
  $perm = if (Test-Path $path -PathType Container) { '(OI)(CI)F' } else { 'F' }
  icacls $path /inheritance:r /grant:r "*${sid}:$perm" "*S-1-5-18:$perm" | Out-Null
}

function Do-Install {
  if (-not $Token -and -not $AllowLogin) {
    # A POC default so the node can start; a real deploy passes -Token or -AllowLogin.
    $Token = -join ((48..57)+(97..102) | Get-Random -Count 48 | ForEach-Object {[char]$_})
    Write-Host "generated a bearer token (no -Token/-AllowLogin given)"
  }
  New-Item -ItemType Directory -Force $ProgDir,$DataDir,$LogDir | Out-Null
  $src = Join-Path $PSScriptRoot "$Name.exe"
  if (-not (Test-Path $src)) { throw "missing $src beside this script" }
  Copy-Item $src $Exe -Force
  Lock-Down $DataDir

  # Build the argument list (flags, not env vars: a Scheduled Task does not inherit them,
  # and WSL-set env vars never cross into a Windows process either).
  $args = @('--port', "$Port", '--public-url', "https://localhost:$ServePort/mcp")
  if ($AllowLogin) { $args += @('--allow-login', $AllowLogin) }
  if ($Token -and -not (Test-Path $TokenF)) { Set-Content -LiteralPath $TokenF -Value $Token -NoNewline -Encoding ascii; Lock-Down $TokenF }
  if (Test-Path $TokenF) { $args += @('--token-file', $TokenF) }
  if (-not $AllowLogin -and -not (Test-Path $TokenF)) { throw "no auth configured" }

  $action    = New-ScheduledTaskAction -Execute $Exe -Argument ($args -join ' ') -WorkingDirectory $ProgDir
  $trigger   = New-ScheduledTaskTrigger -AtLogOn -User $env:USERNAME
  $principal = New-ScheduledTaskPrincipal -UserId "$env:USERDOMAIN\$env:USERNAME" -LogonType Interactive -RunLevel Limited
  $settings  = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries `
                 -ExecutionTimeLimit ([TimeSpan]::Zero) -MultipleInstances IgnoreNew `
                 -RestartCount 3 -RestartInterval (New-TimeSpan -Minutes 1) -StartWhenAvailable
  Register-ScheduledTask -TaskName $Name -TaskPath $TaskPath -Action $action -Trigger $trigger `
    -Principal $principal -Settings $settings -Force | Out-Null
  Write-Host "installed task $TaskName"
  if ($NoStart) { Write-Host "NoStart: task created but not started"; return }
  Start-ScheduledTask -TaskName $Name -TaskPath $TaskPath
  $ok = $false
  foreach ($i in 1..40) {
    try { if ((Invoke-WebRequest -UseBasicParsing -TimeoutSec 1 "http://127.0.0.1:$Port/healthz").Content.Trim() -eq 'ok') { $ok = $true; break } } catch {}
    Start-Sleep -Milliseconds 300
  }
  if (-not $ok) { throw "node did not become healthy; see $LogDir" }
  Write-Host "healthy on http://127.0.0.1:$Port/mcp"
  if (-not $SkipTailscale) {
    $ts = Get-Command tailscale.exe -ErrorAction SilentlyContinue
    if ($ts) { & $ts.Source serve --bg --https=$ServePort "http://127.0.0.1:$Port" | Out-Null; Write-Host "tailscale serve on :$ServePort" }
    else { Write-Host "note: Tailscale not found; install it and run: tailscale serve --bg --https=$ServePort http://127.0.0.1:$Port" }
  }
}

function Do-Uninstall {
  try { Stop-ScheduledTask -TaskName $Name -TaskPath $TaskPath -ErrorAction SilentlyContinue } catch {}
  try { Unregister-ScheduledTask -TaskName $Name -TaskPath $TaskPath -Confirm:$false -ErrorAction SilentlyContinue } catch {}
  Get-Process $Name -ErrorAction SilentlyContinue | Stop-Process -Force
  Remove-Item -Recurse -Force $ProgDir -ErrorAction SilentlyContinue
  if ($Purge) { Remove-Item -Recurse -Force $DataDir -ErrorAction SilentlyContinue; Write-Host "removed program + data (token/grants)" }
  else { Write-Host "removed program; kept data at $DataDir (use -Purge to delete token/grants)" }
}

function Do-Status {
  $t = Get-ScheduledTask -TaskName $Name -TaskPath $TaskPath -ErrorAction SilentlyContinue
  if ($t) { Write-Host "task: $($t.State)"; (Get-ScheduledTaskInfo -TaskName $Name -TaskPath $TaskPath).LastTaskResult | ForEach-Object { Write-Host "lastResult: $_" } }
  else { Write-Host "task: not installed" }
  Write-Host "program: $(if (Test-Path $Exe) {'present'} else {'absent'}); data: $(if (Test-Path $DataDir) {'present'} else {'absent'})"
}

switch ($PSCmdlet.ParameterSetName) {
  'Install'   { Do-Install }
  'Uninstall' { Do-Uninstall }
  default     { Do-Status }
}

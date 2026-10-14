<#  install-winpoc.ps1 - install/uninstall the Windows POC hands node (oab-imcp-winpoc).
    CurrentUser scope, no admin. Mirrors install-linux.sh / install-prebuilt.sh.
    Names are the POC names of spec section 13.2, pending the author's confirmation.

    Usage:
      install-winpoc.ps1 -Install [-Token <str> | -AllowLogin <email>] [-Port 8796] [-ServePort 8445]
                         [-NoBrowser] [-Clients claude,codex,copilot,vscode,gemini | -NoClients]
      install-winpoc.ps1 -Uninstall [-Purge] [-Clients ... | -NoClients]
      install-winpoc.ps1 -Status

    -Install also, unless opted out:
      browser     with Node.js on PATH: @playwright/mcp (pinned) + Chromium, a second logon task
                  serving it on loopback, and the node re-serving browser_* (--upstream)
      clients     every detected local MCP client (Claude Code, Codex CLI, GitHub Copilot CLI,
                  VS Code / Insiders, Gemini CLI): registers this node over loopback with the
                  bearer token; -Clients picks some, -NoClients none, -NoClaudeCode drops claude
    Re-running -Install updates in place and keeps the token.
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
  [switch]$NoBrowser,
  [string[]]$Clients,       # default: every client in Get-ClientTable
  [switch]$NoClients,
  [switch]$NoClaudeCode,    # kept from v0.3: same as leaving claude out of -Clients
  [switch]$NoStart          # for CI/tests: create the task but do not start/serve
)
$ErrorActionPreference = 'Stop'
if ($PSVersionTable.PSVersion.Major -le 5) {
  # Started from a PowerShell 7 prompt, Windows PowerShell inherits pwsh's module paths and
  # loads its incompatible modules (ScheduledTasks goes missing). Drop them.
  $env:PSModulePath = (($env:PSModulePath -split ';') | Where-Object { $_ -and $_ -notmatch '\\PowerShell\\(7[^\\]*\\)?Modules\\?$' }) -join ';'
}
# ---- names (POC, spec 13.2; change here once the author settles them) ---------------------
$Name    = 'oab-imcp-winpoc'
$TaskPath= '\OpenAB-POC\'
$TaskName= "$TaskPath$Name"
$PwName  = "$Name-pw"
$PwPort  = 8797
$PwPkg   = '@playwright/mcp@0.0.82'   # same pin as poc/pw-mcp (macOS) and install-linux.sh
$McpName = $Name                      # server name in every MCP client
$CodexTokenVar = 'OAB_IMCP_WINPOC_TOKEN'  # user env var Codex reads the bearer token from
$ProgDir = Join-Path $env:LOCALAPPDATA 'Programs\OpenAB-POC\imcp-winpoc'
$DataDir = Join-Path $env:LOCALAPPDATA 'oab-imcp-winpoc'
# ------------------------------------------------------------------------------------------
$Exe     = Join-Path $ProgDir "$Name.exe"
$TokenF  = Join-Path $DataDir 'token'
$LogDir  = Join-Path $DataDir 'logs'
$PwDir   = Join-Path $DataDir 'pw-mcp'
$PwScript= Join-Path $ProgDir 'pw-mcp.ps1'
$PwProfile= Join-Path $DataDir 'pw-profile'
$LocalUrl= "http://127.0.0.1:$Port/mcp"
$Summary = New-Object System.Collections.Generic.List[string]   # each section adds its lines

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

function Join-Quoted([string[]]$a) {   # one command line: quote what has spaces
  ($a | ForEach-Object { if ($_ -match '\s') { '"' + $_ + '"' } else { $_ } }) -join ' '
}

function Register-LogonTask([string]$taskName, $action) {
  # Same hardening for both tasks: no time limit, runs on battery, one instance, restarts.
  $trigger   = New-ScheduledTaskTrigger -AtLogOn -User $env:USERNAME
  $principal = New-ScheduledTaskPrincipal -UserId "$env:USERDOMAIN\$env:USERNAME" -LogonType Interactive -RunLevel Limited
  $settings  = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries `
                 -ExecutionTimeLimit ([TimeSpan]::Zero) -MultipleInstances IgnoreNew `
                 -RestartCount 3 -RestartInterval (New-TimeSpan -Minutes 1) -StartWhenAvailable
  Register-ScheduledTask -TaskName $taskName -TaskPath $TaskPath -Action $action -Trigger $trigger `
    -Principal $principal -Settings $settings -Force | Out-Null
  Write-Host "installed task $TaskPath$taskName"
}

function Remove-LogonTask([string]$taskName) {
  try { Stop-ScheduledTask -TaskName $taskName -TaskPath $TaskPath -ErrorAction SilentlyContinue } catch {}
  try { Unregister-ScheduledTask -TaskName $taskName -TaskPath $TaskPath -Confirm:$false -ErrorAction SilentlyContinue } catch {}
}

function Stop-Running {
  # The node, plus Playwright's node/Chromium: stopping the task does not end its children,
  # and a running exe or open profile blocks the copy (update) and -Purge.
  Remove-LogonTask $Name
  Remove-LogonTask $PwName
  $procs = @(Get-Process $Name -ErrorAction SilentlyContinue)
  $marks = @($PwScript, $PwDir, $PwProfile)
  $procs += @(Get-CimInstance Win32_Process -ErrorAction SilentlyContinue |
    Where-Object { $c = $_.CommandLine; $_.ProcessId -ne $PID -and $c -and
                   ($marks | Where-Object { $c.IndexOf($_, [StringComparison]::OrdinalIgnoreCase) -ge 0 }) } |
    ForEach-Object { Get-Process -Id $_.ProcessId -ErrorAction SilentlyContinue })
  $procs | Stop-Process -Force -ErrorAction SilentlyContinue
  $procs | Wait-Process -Timeout 15 -ErrorAction SilentlyContinue   # Stop-Process returns before exit
}

function Wait-Http([string]$url, [int]$tries, [scriptblock]$okWhen) {
  foreach ($i in 1..$tries) {
    try { $r = Invoke-WebRequest -UseBasicParsing -TimeoutSec 2 $url; if (& $okWhen $r) { return $true } }
    catch { if ($_.Exception.Response) { if (& $okWhen $_.Exception.Response) { return $true } } }
    Start-Sleep -Milliseconds 500
  }
  return $false
}

function Find-Exe([string]$name) {
  # claude.exe / codex.cmd / code.cmd ...; like node/npm, skip npm's .ps1 shims.
  Get-Command $name -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
}

function Invoke-Quiet([string]$exe, [string[]]$argv, [string]$log) {
  # Run a native tool with its output in $log (and in $script:LastOut); returns the exit
  # code. Windows PowerShell turns redirected native stderr into errors, so relax the
  # preference for the call.
  $ErrorActionPreference = 'Continue'
  "> $exe $(($argv -join ' ') -replace 'Bearer \S+', 'Bearer ***')" | Out-File -Append -Encoding utf8 $log
  $script:LastOut = & $exe @argv *>&1 | Out-String
  $rc = $LASTEXITCODE
  $script:LastOut | Out-File -Append -Encoding utf8 $log
  return $rc
}

# ---- browser: @playwright/mcp as a second logon task, re-served by the node ---------------
function Install-Browser {
  # Returns $true when the node should get --upstream. Any failure only warns.
  if ($NoBrowser) { Remove-LogonTask $PwName; $Summary.Add("  browser : off (-NoBrowser)"); return $false }
  # npm is npm.cmd from the Node.js installer, npm.exe from Volta; skip npm.ps1 shims.
  $node = Get-Command node -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
  $npm  = Get-Command npm -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
  if (-not $node -or -not $npm) {
    Remove-LogonTask $PwName
    $Summary.Add("  browser : skipped - Node.js not found; install it (https://nodejs.org) and re-run setup")
    return $false
  }
  $log = Join-Path $LogDir 'pw-install.log'
  Write-Host "browser: installing $PwPkg and Chromium (log: $log)"
  try {
    New-Item -ItemType Directory -Force $PwDir | Out-Null
    if (-not (Test-Path (Join-Path $PwDir 'package.json'))) {
      Set-Content -LiteralPath (Join-Path $PwDir 'package.json') -Value '{ "private": true }' -Encoding ascii
    }
    Push-Location $PwDir
    try {
      $rc = Invoke-Quiet $npm.Source @('install', '--save-exact', '--no-audit', '--no-fund', $PwPkg) $log
      if ($rc -ne 0) { throw "npm install $PwPkg failed (exit $rc)" }
      $pwCli = Join-Path $PwDir 'node_modules\.bin\playwright.cmd'
      if (-not (Test-Path $pwCli)) { throw "playwright CLI missing after npm install" }
      $rc = Invoke-Quiet $pwCli @('install', 'chromium') $log
      if ($rc -ne 0) { throw "playwright install chromium failed (exit $rc)" }
    } finally { Pop-Location }
    if (-not (Test-Path (Join-Path $PwDir 'node_modules\.bin\playwright-mcp.cmd'))) { throw "playwright-mcp missing after npm install" }

    $ps = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
    $a = @('-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-WindowStyle', 'Hidden',
           '-File', $PwScript, '-NodeDir', (Split-Path -Parent $node.Source), '-Port', "$PwPort", '-Base', $DataDir)
    $action = New-ScheduledTaskAction -Execute $ps -Argument (Join-Quoted $a) -WorkingDirectory $ProgDir
    Register-LogonTask $PwName $action
  } catch {
    Write-Warning "browser tools not set up: $($_.Exception.Message) (see $log)"
    Remove-LogonTask $PwName
    $Summary.Add("  browser : FAILED - $($_.Exception.Message); core tools still work (log: $log)")
    return $false
  }
  if ($NoStart) { $Summary.Add("  browser : task $TaskPath$PwName (not started: -NoStart)"); return $true }
  Start-ScheduledTask -TaskName $PwName -TaskPath $TaskPath
  # Any HTTP answer means it is listening (GET /mcp is a 4xx); the node tolerates a late start.
  $up = Wait-Http "http://127.0.0.1:$PwPort/mcp" 60 { param($r) $true }
  if ($up) { $Summary.Add("  browser : browser_* via Playwright MCP on 127.0.0.1:$PwPort (task $TaskPath$PwName)") }
  else {
    Write-Warning "Playwright MCP did not answer on 127.0.0.1:$PwPort yet; browser_* appear once it does"
    $Summary.Add("  browser : task $TaskPath$PwName installed, not answering yet (log: $LogDir)")
  }
  return $true
}

# ---- MCP clients: the README's client pattern (`mcp add … --header "Authorization: Bearer …"`,
# shown there for kiro-cli) applied to each local client over loopback. One row per client;
# install and uninstall walk the same table, and any failure only warns. Pick is the name
# -Clients selects it by (both VS Code rows answer to 'vscode'). A CLI row confirms by
# finding the server name in its Verify output.
function Get-ClientTable([string]$tok) {
  $hdr = "Authorization: Bearer $tok"
  @(
    @{ Pick = 'claude'; Label = 'Claude Code'; Exe = 'claude'; Where = '~\.claude.json'; Paste = $true
       Remove = @('mcp', 'remove', '--scope', 'user', $McpName)
       Add    = @('mcp', 'add', '--transport', 'http', '--scope', 'user', $McpName, $LocalUrl, '--header', $hdr)   # --header is variadic: last
       Verify = @('mcp', 'get', $McpName) }
    @{ Pick = 'codex'; Label = 'Codex CLI'; Exe = 'codex'; Where = "user env var $CodexTokenVar"; TokenVar = $CodexTokenVar
       Remove = @('mcp', 'remove', $McpName)
       Add    = @('mcp', 'add', $McpName, '--url', $LocalUrl, '--bearer-token-env-var', $CodexTokenVar)   # its CLI takes no headers
       Verify = @('mcp', 'get', $McpName) }
    @{ Pick = 'copilot'; Label = 'GitHub Copilot CLI'; Exe = 'copilot'; Where = '~\.copilot\mcp-config.json'
       Remove = @('mcp', 'remove', $McpName)
       Add    = @('mcp', 'add', '--transport', 'http', $McpName, $LocalUrl, '--header', $hdr)
       Verify = @('mcp', 'get', $McpName) }
    @{ Pick = 'gemini'; Label = 'Gemini CLI'; Exe = 'gemini'; Where = '~\.gemini\settings.json'
       Remove = @('mcp', 'remove', '-s', 'user', $McpName)
       Add    = @('mcp', 'add', '-s', 'user', '--transport', 'http', '--header', $hdr, $McpName, $LocalUrl)   # order as in its docs
       Verify = @('mcp', 'list') }   # gemini has no `mcp get`
    # VS Code has no removal CLI, and --add-mcp's JSON loses its quotes on the way through
    # Windows PowerShell to code.cmd, so the user mcp.json is edited directly.
    @{ Pick = 'vscode'; Label = 'VS Code'; Exe = 'code'; McpJson = (Join-Path $env:APPDATA 'Code\User\mcp.json') }
    @{ Pick = 'vscode'; Label = 'VS Code Insiders'; Exe = 'code-insiders'; McpJson = (Join-Path $env:APPDATA 'Code - Insiders\User\mcp.json') }
  )
}

function Get-SelectedClients {
  # All clients unless -Clients names some. Through powershell -File, -Clients arrives as one
  # "a,b" string; split it either way.
  if ($NoClients -and $Clients) { throw "-Clients and -NoClients: pass one, not both" }
  if ($NoClients) { return @() }
  $known = @(Get-ClientTable '' | ForEach-Object { $_.Pick } | Select-Object -Unique)
  $sel = if ($Clients) { @($Clients | ForEach-Object { $_ -split ',' } | ForEach-Object { $_.Trim().ToLower() } | Where-Object { $_ }) } else { $known }
  foreach ($c in $sel) { if ($known -notcontains $c) { throw "unknown client '$c' in -Clients (known: $($known -join ', '))" } }
  if ($NoClaudeCode) { $sel = @($sel | Where-Object { $_ -ne 'claude' }) }
  return $sel
}

function Read-McpJson([string]$path) {
  # The parsed file, or $null when there is none; throws when it is not plain JSON (VS Code
  # allows comments, Windows PowerShell's parser does not), so it is never rewritten then.
  # VS Code writes UTF-8 without a BOM, which Windows PowerShell would read as ANSI.
  if (-not (Test-Path -LiteralPath $path)) { return $null }
  $raw = Get-Content -Raw -Encoding UTF8 -LiteralPath $path
  if (-not $raw -or -not $raw.Trim()) { return $null }
  return ($raw | ConvertFrom-Json)
}

function Write-McpJson([string]$path, $j) {
  New-Item -ItemType Directory -Force (Split-Path -Parent $path) | Out-Null
  $bak = "$path.$Name.bak"   # the user's original, once
  if ((Test-Path -LiteralPath $path) -and -not (Test-Path -LiteralPath $bak)) { Copy-Item -LiteralPath $path $bak }
  [IO.File]::WriteAllText($path, ($j | ConvertTo-Json -Depth 32), (New-Object Text.UTF8Encoding $false))
}

function Register-VsCode($row, [string]$tok) {
  # Returns '' when the server is in the file afterwards, else why not.
  try { $j = Read-McpJson $row.McpJson } catch { return "could not parse $($row.McpJson) (comments?); left it alone" }
  try {
    if ($null -eq $j) { $j = [pscustomobject]@{ servers = [pscustomobject]@{}; inputs = @() } }
    if ($null -eq $j.PSObject.Properties['servers'] -or $null -eq $j.servers) { $j | Add-Member -Force -NotePropertyName servers -NotePropertyValue ([pscustomobject]@{}) }
    $j.servers.PSObject.Properties.Remove($McpName)
    $j.servers | Add-Member -NotePropertyName $McpName -NotePropertyValue ([pscustomobject]@{
      type = 'http'; url = $LocalUrl; headers = [pscustomobject]@{ Authorization = "Bearer $tok" } })
    Write-McpJson $row.McpJson $j
    if ($null -eq (Read-McpJson $row.McpJson).servers.PSObject.Properties[$McpName]) { return "the entry is missing after writing $($row.McpJson)" }
    return ''
  } catch { return "could not update $($row.McpJson): $($_.Exception.Message)" }
}

function Unregister-VsCode($row) {
  try {
    $j = Read-McpJson $row.McpJson
    if ($null -eq $j -or $null -eq $j.servers -or $null -eq $j.servers.PSObject.Properties[$McpName]) { return }
    $j.servers.PSObject.Properties.Remove($McpName)
    Write-McpJson $row.McpJson $j
    Write-Host "removed $($row.Label) server $McpName"
  } catch { Write-Warning "$($row.Label): could not update $($row.McpJson); remove '$McpName' from it by hand" }
}

function Register-Clients([string[]]$sel) {
  if ($NoClients) { $Summary.Add("  MCP clients: skipped (-NoClients)"); return }
  if (-not $sel) { $Summary.Add("  MCP clients: none selected"); return }
  if ($AllowLogin -or -not (Test-Path $TokenF)) {
    $Summary.Add("  MCP clients: not registered - -AllowLogin needs a Tailscale identity, and the loopback URL")
    $Summary.Add("               carries no Tailscale identity; register the tailnet URL by hand")
    return
  }
  $tok = (Get-Content -Raw $TokenF).Trim()
  $log = Join-Path $LogDir 'clients.log'
  $missing = @(); $stored = @(); $restart = $false
  foreach ($row in @(Get-ClientTable $tok | Where-Object { $sel -contains $_.Pick })) {
    $exe = Find-Exe $row.Exe
    if (-not $exe) {
      $missing += $row.Label
      if ($row.Paste) {
        $Summary.Add("  $($row.Label): not found; after installing it, run:")
        $Summary.Add("      $($row.Exe) $(Join-Quoted $row.Add)")
      }
      continue
    }
    if ($row.McpJson) {
      $why = Register-VsCode $row $tok
      if (-not $why) { $Summary.Add("  $($row.Label): registered '$McpName' ($($row.McpJson))"); $stored += $row.McpJson }
      else {
        Write-Warning "$($row.Label): $why"
        $Summary.Add("  $($row.Label): $why; add under ""servers"":")
        $Summary.Add("      ""$McpName"": { ""type"": ""http"", ""url"": ""$LocalUrl"", ""headers"": { ""Authorization"": ""Bearer $tok"" } }")
      }
      continue
    }
    [void](Invoke-Quiet $exe.Source $row.Remove $log)   # re-runs replace, never duplicate
    if ($row.TokenVar) { [Environment]::SetEnvironmentVariable($row.TokenVar, $tok, 'User') }
    $rc = Invoke-Quiet $exe.Source $row.Add $log
    if ($rc -ne 0) {
      if ($row.TokenVar) { [Environment]::SetEnvironmentVariable($row.TokenVar, $null, 'User') }   # no token left behind
      Write-Warning "$($row.Exe) mcp add failed (exit $rc; see $log)"
      $Summary.Add("  $($row.Label): FAILED to register (see $log)")
      continue
    }
    $rc = Invoke-Quiet $exe.Source $row.Verify $log
    $confirmed = $rc -eq 0 -and $script:LastOut -match [regex]::Escape($McpName)
    if (-not $confirmed) { Write-Warning "$($row.Exe) did not confirm $McpName (exit $rc; see $log)" }
    $Summary.Add("  $($row.Label): registered '$McpName' ($($row.Where))$(if (-not $confirmed) { ' - not confirmed, see ' + $log })")
    $stored += $row.Where
    if ($row.TokenVar) { $restart = $true }
  }
  if ($missing) { $Summary.Add("  not found: $($missing -join ', ')") }
  if ($stored) { $Summary.Add("  note: the bearer token is stored in plain text in: $($stored -join '; ')") }
  if ($restart) { $Summary.Add("        restart open terminals so Codex sees $CodexTokenVar") }
}

function Unregister-Clients {
  # Every client, whatever -Clients said at install time; -NoClients leaves them all alone.
  if ($NoClients) { return }
  $log = Join-Path $env:TEMP "$Name-uninstall.log"
  foreach ($row in @(Get-ClientTable '')) {
    if ($row.TokenVar) { [Environment]::SetEnvironmentVariable($row.TokenVar, $null, 'User') }
    if ($row.McpJson) { Unregister-VsCode $row; continue }   # the file may outlive `code` on PATH
    $exe = Find-Exe $row.Exe
    if (-not $exe) { continue }
    if ((Invoke-Quiet $exe.Source $row.Remove $log) -eq 0) { Write-Host "removed $($row.Label) server $McpName" }
  }
}

function Do-Install {
  $sel = @(Get-SelectedClients)   # a bad -Clients fails before anything changes
  if (-not $Token -and -not $AllowLogin -and -not (Test-Path $TokenF)) {
    # A POC default so the node can start; a real deploy passes -Token or -AllowLogin.
    $Token = -join ((48..57)+(97..102) | Get-Random -Count 48 | ForEach-Object {[char]$_})
    Write-Host "generated a bearer token (no -Token/-AllowLogin given)"
  }
  if ($Token -and (Test-Path $TokenF)) { Write-Host "note: keeping the existing token at $TokenF; -Token is ignored (-Uninstall -Purge first to change it)" }
  $src = Join-Path $PSScriptRoot "$Name.exe"
  if (-not (Test-Path $src)) { throw "missing $src beside this script" }
  Stop-Running   # an update: the running exe cannot be overwritten
  New-Item -ItemType Directory -Force $ProgDir,$DataDir,$LogDir | Out-Null
  Copy-Item $src $Exe -Force
  foreach ($f in 'install-winpoc.ps1', 'pw-mcp.ps1') {
    $p = Join-Path $PSScriptRoot $f
    if (Test-Path $p) { Copy-Item $p (Join-Path $ProgDir $f) -Force }   # pw task + offline -Uninstall
  }
  Lock-Down $DataDir

  # Build the argument list (flags, not env vars: a Scheduled Task does not inherit them,
  # and WSL-set env vars never cross into a Windows process either).
  $nodeArgs = @('--port', "$Port", '--public-url', "https://localhost:$ServePort/mcp")
  if ($AllowLogin) { $nodeArgs += @('--allow-login', $AllowLogin) }
  if ($Token -and -not (Test-Path $TokenF)) { Set-Content -LiteralPath $TokenF -Value $Token -NoNewline -Encoding ascii; Lock-Down $TokenF }
  if (Test-Path $TokenF) { $nodeArgs += @('--token-file', $TokenF) }
  if (-not $AllowLogin -and -not (Test-Path $TokenF)) { throw "no auth configured" }
  if (Install-Browser) { $nodeArgs += @('--upstream', "browser=http://127.0.0.1:$PwPort/mcp") }

  $action = New-ScheduledTaskAction -Execute $Exe -Argument (Join-Quoted $nodeArgs) -WorkingDirectory $ProgDir
  Register-LogonTask $Name $action
  if ($NoStart) { Write-Host "NoStart: task created but not started"; return }
  Start-ScheduledTask -TaskName $Name -TaskPath $TaskPath
  $ok = Wait-Http "http://127.0.0.1:$Port/healthz" 40 { param($r) $r.StatusCode -eq 200 -and "$($r.Content)".Trim() -eq 'ok' }
  if (-not $ok) { throw "node did not become healthy; see $LogDir" }
  $serveUrl = $LocalUrl
  if (-not $SkipTailscale) {
    $ts = Get-Command tailscale.exe -ErrorAction SilentlyContinue
    if ($ts) {
      & $ts.Source serve --bg --https=$ServePort "http://127.0.0.1:$Port" | Out-Null
      $dns = (& $ts.Source status --json 2>$null | ConvertFrom-Json).Self.DNSName.TrimEnd('.')
      if ($dns) { $serveUrl = "https://${dns}:$ServePort/mcp" }
    } else {
      Write-Host "note: Tailscale not found; for remote access install it, then run:"
      Write-Host "      tailscale serve --bg --https=$ServePort http://127.0.0.1:$Port"
    }
  }
  Register-Clients $sel

  Write-Host ""
  Write-Host "================ ready ================"
  Write-Host "  MCP URL : $serveUrl"
  if (Test-Path $TokenF) { Write-Host "  token   : $(Get-Content -Raw $TokenF)  (also at $TokenF)" }
  elseif ($AllowLogin)   { Write-Host "  auth    : Tailscale login $AllowLogin" }
  Write-Host "  autostart: logon task $TaskName (runs now and every sign-in)"
  foreach ($line in $Summary) { Write-Host $line }
  Write-Host "======================================"
}

function Do-Uninstall {
  Stop-Running
  Unregister-Clients
  Remove-Item -Recurse -Force $ProgDir -ErrorAction SilentlyContinue
  if ($Purge) {
    # token, grants, logs, and the browser pieces: pw-mcp (install), pw-profile, pw-output
    Remove-Item -Recurse -Force $DataDir -ErrorAction SilentlyContinue
    Write-Host "removed program + data (token/grants/browser profile)"
  }
  else { Write-Host "removed program; kept data at $DataDir (use -Purge to delete token/grants/browser profile)" }
}

function Do-Status {
  foreach ($n in $Name, $PwName) {
    $t = Get-ScheduledTask -TaskName $n -TaskPath $TaskPath -ErrorAction SilentlyContinue
    if ($t) { Write-Host "task $n : $($t.State); lastResult: $((Get-ScheduledTaskInfo -TaskName $n -TaskPath $TaskPath).LastTaskResult)" }
    else { Write-Host "task $n : not installed" }
  }
  Write-Host "program: $(if (Test-Path $Exe) {'present'} else {'absent'}); data: $(if (Test-Path $DataDir) {'present'} else {'absent'})"
}

switch ($PSCmdlet.ParameterSetName) {
  'Install'   { Do-Install }
  'Uninstall' { Do-Uninstall }
  default     { Do-Status }
}

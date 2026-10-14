# Windows POC: VM acceptance with Claude Code

How to run the clean-machine acceptance of the one-step setup in a Hyper-V VM, with Claude
Code inside the VM doing the checking. Why a VM, and what the other options cover:
[`docs/requirements/windows-setup-testing.md`](../../../docs/requirements/windows-setup-testing.md).
The items here are the open ones in [`MANUAL-ACCEPTANCE.md`](MANUAL-ACCEPTANCE.md).

The split: **you** do what needs eyes or a sign-out (preparing the VM, double-clicking,
watching SmartScreen, signing out and in). **Claude Code in the VM** runs the commands and
reports. Each test below is one prompt, pasted into a **fresh** Claude Code session, and ends
with a fixed report. Paste the reports back to the developer session to update the docs.

| Test | Covers | Who |
|---|---|---|
| [P0 Baseline](#p0-baseline-read-only) | the VM is really clean: standard user, no VC++ runtime, no Node.js, no prior install | Claude |
| [H1 Double-click and SmartScreen](#h1-double-click-and-smartscreen-human) | install from a downloaded ZIP the way a user does; what SmartScreen shows | You |
| [P1 Clean install, no Node.js](#p1-clean-install-no-nodejs) | install from the real release as a standard user; exe runs without the VC++ runtime; Mark of the Web removed | Claude |
| [P2 Browser tools and update](#p2-browser-tools-and-update) | Node.js present: Playwright task, `--upstream`, `browser_*`; re-run keeps the token | Claude |
| [P3 The tools from Claude's side](#p3-the-tools-from-claudes-side-new-session) | Claude Code really uses the node: `sys_info`, `screenshot`, `powershell`, `browser_navigate` | Claude |
| [P4 Other MCP clients](#p4-other-mcp-clients) | real Codex, Copilot CLI, Gemini CLI (and VS Code) registration | Claude |
| [P5 Autostart after sign-in](#p5-autostart-after-sign-in) | both logon tasks start by themselves | You sign out and in, then Claude |
| [P6 Uninstall and purge](#p6-uninstall-and-purge) | everything is removed again, clients included | Claude |

## Prepare the VM (once)

1. **Guest**: Windows 10 or 11, freshly installed, with internet access to github.com.
2. **Accounts**: one administrator account (for installing tools when asked) and one
   **standard user** for every test. Sign in as the standard user for everything below.
3. **Do not install** Node.js, Visual Studio, or the VC++ redistributable yet.
4. **Git for Windows** (the per-user installer is fine) if Claude Code asks for it, then
   **Claude Code** as the standard user, and sign in to it:

   ```powershell
   irm https://claude.ai/install.ps1 | iex
   claude            # sign in once, then /exit
   ```

5. **The repo**, on the branch that has the setup:

   ```powershell
   git clone https://github.com/pro5251/instance-mcp $HOME\instance-mcp
   cd $HOME\instance-mcp; git checkout feat/windows-setup
   ```

6. **Checkpoint** on the host (administrator PowerShell), so every round can start clean:

   ```powershell
   Checkpoint-VM -Name <vm> -SnapshotName clean
   # later: Restore-VMCheckpoint -VMName <vm> -Name clean -Confirm:$false
   ```

**Running a prompt**: in the VM, `cd $HOME\instance-mcp`, start `claude`, paste the prompt.
Use the default permission mode and approve the commands as they come (they only read state,
run setup, and call the node on loopback). Start a **new** session for each prompt; P3 must be
a new session because Claude Code loads MCP servers only at start.

**Order**: P0 → H1 or P1 → (install Node.js LTS with the admin account; take a checkpoint
`with-node`) → P2 → P3 → (install the other clients, see P4) → P4 → P5 → P6.

## Rules for every prompt

Each prompt below already includes these; they are here so you know what Claude is told:

- Do not edit any file in the repo and do not try to fix the product. If a check fails,
  collect the evidence (command output, `%LOCALAPPDATA%\oab-imcp-winpoc\logs\*`) and continue.
- Never run anything elevated, never change system settings.
- Hide secrets in the report: replace every bearer token with `***`.
- End with the report format: a table `# | check | expected | actual | PASS/FAIL/N/A`, then
  the raw evidence that matters.

## P0 Baseline (read-only)

```text
You are checking that this Windows VM is a clean test machine for the instance-mcp Windows
setup. Read-only: change nothing, run nothing elevated, do not edit repo files.

Run these in PowerShell and record the results:
1. OS: (Get-CimInstance Win32_OperatingSystem) Caption, Version, BuildNumber.
2. Current user and admin status:
   - whoami
   - ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
   - whoami /groups | Select-String 'S-1-5-32-544'   (present = member of Administrators)
   Expected: not elevated and NOT a member of Administrators.
3. VC++ runtime absent:
   - Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\VisualStudio\14.0\VC\Runtimes\X64' -EA 0
   - Test-Path C:\Windows\System32\vcruntime140.dll
   Expected: no registry key and False.
4. Node.js absent: Get-Command node, npm -EA 0. Expected: nothing.
5. No earlier install: Get-ScheduledTask -TaskPath '\OpenAB-POC\' -EA 0;
   Test-Path "$env:LOCALAPPDATA\oab-imcp-winpoc"; Test-Path "$env:LOCALAPPDATA\Programs\OpenAB-POC";
   [Environment]::GetEnvironmentVariable('OAB_IMCP_WINPOC_TOKEN','User'). Expected: none.
6. Clients present: Get-Command claude, codex, copilot, gemini, code -EA 0 | Select Name, Source.
7. Network: (Invoke-RestMethod https://api.github.com/repos/pro5251/instance-mcp/releases?per_page=5).tag_name
   Expected: includes v0.8.0-winpoc.1 (or newer).

Report: a table "# | check | expected | actual | PASS/FAIL/N/A", then a one-line verdict:
"clean" or "not clean because ...".
```

## H1 Double-click and SmartScreen (human)

Do this instead of P1 for the first clean round (restore `clean` first), so the install
happens exactly as a user does it:

1. In the VM's browser, download
   `https://github.com/pro5251/instance-mcp/archive/refs/heads/feat/windows-setup.zip`
   and extract it (Explorer → Extract all). The files now carry the "downloaded from the
   internet" mark.
2. Double-click `scripts\setup-winpoc.cmd` in the extracted folder. Write down **every**
   prompt Windows shows (SmartScreen "Windows protected your PC", "Open File – Security
   Warning", Smart App Control) and what you clicked.
3. Copy the summary between `==== ready ====` and `======` from the window.
4. Then run P1 **from step 3 on** (skip its setup run) in a new Claude session, so Claude
   checks the result, and tell it the folder you extracted to.

## P1 Clean install, no Node.js

```text
You are running the acceptance test "clean install, no Node.js" for the instance-mcp Windows
setup in this VM, as the current standard user. Do not edit repo files, do not fix the
product, run nothing elevated. Replace bearer tokens with *** in everything you report.
If a check fails, collect evidence (output, files under %LOCALAPPDATA%\oab-imcp-winpoc\logs)
and continue with the next check.

1. From the repo root run (no -ApiBase: this uses the real GitHub release):
   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\setup-winpoc.ps1
   Record the exit code ($LASTEXITCODE) and the whole summary block.
   Expected: exit 0; "release v0.8.0-winpoc.1" (or newer); "sha256 ok"; a "ready" summary;
   browser line says Node.js not found; Claude Code line says registered.
2. Not elevated: confirm step 1 ran without any UAC prompt and the user is not an admin.
3. Tasks: Get-ScheduledTask -TaskPath '\OpenAB-POC\' | Select TaskName, State
   Expected: oab-imcp-winpoc Running (no -pw task, since there is no Node.js).
   Settings: (Get-ScheduledTask -TaskPath '\OpenAB-POC\' -TaskName oab-imcp-winpoc).Settings |
   Select ExecutionTimeLimit, DisallowStartIfOnBatteries, MultipleInstances
   Expected: PT0S, False, IgnoreNew.
4. Health: curl.exe -s http://127.0.0.1:8796/healthz  -> ok
5. Tools with the token:
   $t = (Get-Content "$env:LOCALAPPDATA\oab-imcp-winpoc\token").Trim()
   (Invoke-RestMethod -Method Post http://127.0.0.1:8796/mcp -Headers @{Authorization="Bearer $t"} -ContentType application/json -Body '{"jsonrpc":"2.0","id":1,"method":"tools/list"}').result.tools.name
   Expected: exactly 9: sys_info screenshot mouse key powershell exec_start exec_poll exec_list exec_cancel.
6. Wrong token is refused: the same call with "Bearer WRONG" must fail with HTTP 401.
7. Token file DACL: icacls "$env:LOCALAPPDATA\oab-imcp-winpoc\token"
   Expected: only the current user and SYSTEM; no Users, Everyone, Authenticated Users.
8. The exe runs without the VC++ runtime: re-check that
   HKLM:\SOFTWARE\Microsoft\VisualStudio\14.0\VC\Runtimes\X64 does not exist and
   C:\Windows\System32\vcruntime140.dll is absent, while step 4 is ok.
9. Mark of the Web removed:
   Get-Item "$env:LOCALAPPDATA\Programs\OpenAB-POC\imcp-winpoc\oab-imcp-winpoc.exe" -Stream Zone.Identifier -EA 0
   Get-ChildItem "$env:TEMP\oab-imcp-winpoc-setup" -Recurse -File | ForEach-Object { Get-Item $_.FullName -Stream Zone.Identifier -EA 0 }
   Expected: no Zone.Identifier stream anywhere.
10. Claude Code registration: claude mcp get oab-imcp-winpoc
    Expected: User config, http://127.0.0.1:8796/mcp, Status Connected (if the first call says
    "tools fetch failed / timed out", wait 5 s and run it once more; report both).

Report: table "# | check | expected | actual | PASS/FAIL/N/A", the summary block from step 1
(tokens as ***), and the content of any log that shows an error.
```

## P2 Browser tools and update

Before: with the administrator account, install Node.js LTS (the MSI asks for admin
credentials; that is fine, only setup must run as the standard user). Sign back in as the
standard user and open a new terminal.

```text
You are running the acceptance test "browser tools and update" for the instance-mcp Windows
setup. Node.js has just been installed; setup was already run once without it. Do not edit
repo files, do not fix the product, run nothing elevated, replace tokens with ***.
On failure, collect evidence (including %LOCALAPPDATA%\oab-imcp-winpoc\logs\pw-install.log)
and continue.

1. Record the current token's first 6 characters:
   (Get-Content "$env:LOCALAPPDATA\oab-imcp-winpoc\token").Trim().Substring(0,6)
2. Re-run setup from the repo root:
   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\setup-winpoc.ps1
   Expected: exit 0; browser line says "browser_* via Playwright MCP on 127.0.0.1:8797";
   no "generated a bearer token" line.
3. Token kept: the first 6 characters equal step 1.
4. Tasks: both oab-imcp-winpoc and oab-imcp-winpoc-pw exist and run; the -pw task has the same
   settings (PT0S, False, IgnoreNew); its action is powershell.exe with -WindowStyle Hidden
   and ...\imcp-winpoc\pw-mcp.ps1.
5. The node forwards the browser: (Get-ScheduledTask -TaskPath '\OpenAB-POC\' -TaskName oab-imcp-winpoc).Actions[0].Arguments
   Expected: contains --upstream browser=http://127.0.0.1:8797/mcp
6. Pinned version: Get-Content "$env:LOCALAPPDATA\oab-imcp-winpoc\pw-mcp\package.json"
   Expected: "@playwright/mcp": "0.0.82" (exact, no ^).
7. tools/list with the token (as in the docs): 9 local tools plus browser_* tools (count them;
   expect about 30). If browser_* are missing, retry for up to 30 s.
8. claude mcp get oab-imcp-winpoc -> Connected.

Report: table "# | check | expected | actual | PASS/FAIL/N/A", the summary block (tokens ***),
and pw-install.log errors if any.
```

## P3 The tools from Claude's side (new session)

Start a **new** Claude Code session after P2 (it must load `oab-imcp-winpoc` at start).

```text
You are testing the instance-mcp Windows node from the client side. This session should have
an MCP server called oab-imcp-winpoc (registered by setup). Do not edit repo files, run
nothing elevated.

1. Run /mcp (or `claude mcp list` in a shell) and confirm oab-imcp-winpoc is connected.
   List the tools you can see from it.
2. Call its sys_info tool. Report the OS, display info, and agent name it returns.
3. Call its screenshot tool and describe in one sentence what is on the screen.
4. Call its powershell tool with: whoami; $PSVersionTable.PSVersion.ToString()
   The user must be the current standard user.
5. Call browser_navigate with https://example.com, then browser_snapshot.
   Expected: the page title / heading is "Example Domain". A Chromium window opens on the
   desktop; that is expected.
6. Call browser_close.

Report: table "# | check | expected | actual | PASS/FAIL/N/A". For any failure, quote the
tool error exactly.
```

## P4 Other MCP clients

Before: as the standard user, install the client CLIs (they go into `%APPDATA%\npm`, no admin
needed), and optionally VS Code with its **user** installer (it puts `code` on PATH):

```powershell
npm install -g @openai/codex @github/copilot @google/gemini-cli
```

Open a new terminal, then:

```text
You are running the acceptance test "other MCP clients" for the instance-mcp Windows setup.
Codex CLI, GitHub Copilot CLI and Gemini CLI were just installed (and maybe VS Code). None of
them needs to be signed in for this test. Do not edit repo files, do not fix the product, run
nothing elevated, replace tokens with ***.

1. Record which are on PATH: Get-Command claude, codex, copilot, gemini, code, code-insiders -EA 0
2. Re-run setup from the repo root:
   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\setup-winpoc.ps1
   Expected: exit 0; one "registered 'oab-imcp-winpoc'" line per client found; no FAILED line;
   a "not found:" line listing only the clients that are really missing.
3. Codex: codex mcp get oab-imcp-winpoc
   Expected: streamable_http, url http://127.0.0.1:8796/mcp, bearer_token_env_var OAB_IMCP_WINPOC_TOKEN.
   [Environment]::GetEnvironmentVariable('OAB_IMCP_WINPOC_TOKEN','User') is set (report only "set").
4. Copilot CLI: copilot mcp get oab-imcp-winpoc
   Expected: Type http, URL http://127.0.0.1:8796/mcp, an Authorization header, Source User.
5. Gemini CLI: gemini mcp list
   Expected: oab-imcp-winpoc listed with http://127.0.0.1:8796/mcp. Also show the entry in
   $HOME\.gemini\settings.json (tokens ***): it should use httpUrl (or url) and a headers block.
   This is the first real run of the Gemini commands; report the exact output.
6. VS Code (only if code or code-insiders is on PATH): show servers.oab-imcp-winpoc in
   $env:APPDATA\Code\User\mcp.json (or "Code - Insiders"): type http, the URL, a headers
   block. Any other servers in that file must still be there.
7. Run the clients log check: Get-Content "$env:LOCALAPPDATA\oab-imcp-winpoc\logs\clients.log" -Tail 60
   (tokens are already masked there); quote any error lines.

Report: table "# | client | expected | actual | PASS/FAIL/N/A", plus the exact Gemini output.
```

## P5 Autostart after sign-in

Before: sign out of the standard user (Start → user → Sign out), sign back in, wait one
minute. Do not run setup. Open a new terminal and a new Claude session.

```text
You are checking that the instance-mcp Windows node started by itself at sign-in. Do not run
setup, do not edit repo files, run nothing elevated.

1. Sign-in time: (Get-Process explorer | Sort-Object StartTime | Select-Object -First 1).StartTime
2. Tasks: foreach ($n in 'oab-imcp-winpoc','oab-imcp-winpoc-pw') {
     $i = Get-ScheduledTaskInfo -TaskPath '\OpenAB-POC\' -TaskName $n -EA 0
     "$n state=$((Get-ScheduledTask -TaskPath '\OpenAB-POC\' -TaskName $n -EA 0).State) lastRun=$($i.LastRunTime) result=$($i.LastTaskResult)" }
   Expected: both Running, LastRunTime after the sign-in time in step 1 (within a minute or two).
3. curl.exe -s http://127.0.0.1:8796/healthz -> ok
4. tools/list with the token: 9 local tools plus browser_* (retry for up to 30 s).
5. claude mcp get oab-imcp-winpoc -> Connected.

Report: table "# | check | expected | actual | PASS/FAIL/N/A" with the times you compared.
```

## P6 Uninstall and purge

```text
You are running the acceptance test "uninstall and purge" for the instance-mcp Windows setup.
Do not edit repo files, run nothing elevated, replace tokens with ***.

Part A, uninstall keeping data:
1. powershell -NoProfile -ExecutionPolicy Bypass -File scripts\setup-winpoc.ps1 -Uninstall
   Expected: exit 0; "removed ... server oab-imcp-winpoc" for each registered client;
   "kept data at ...".
2. Get-ScheduledTask -TaskPath '\OpenAB-POC\' -EA 0 -> nothing; no oab-imcp-winpoc or node
   process for this user; curl.exe -s -m 3 http://127.0.0.1:8796/healthz fails.
3. %LOCALAPPDATA%\Programs\OpenAB-POC\imcp-winpoc is gone; %LOCALAPPDATA%\oab-imcp-winpoc\token
   still exists.
4. Clients: claude mcp get oab-imcp-winpoc, codex mcp get oab-imcp-winpoc,
   copilot mcp get oab-imcp-winpoc, gemini mcp list (only those installed): the server is gone.
   VS Code mcp.json (if any): no servers.oab-imcp-winpoc, other servers kept.
   [Environment]::GetEnvironmentVariable('OAB_IMCP_WINPOC_TOKEN','User') is empty.

Part B, purge:
5. Install again quietly, then purge:
   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\setup-winpoc.ps1 -NoBrowser
   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\setup-winpoc.ps1 -Uninstall -Purge
   Expected: exit 0 both; "removed program + data".
6. %LOCALAPPDATA%\oab-imcp-winpoc is gone (token, logs, pw-mcp, pw-profile, pw-output);
   no tasks; clients clean as in step 4.
7. Left alone on purpose (report, not a failure): %LOCALAPPDATA%\ms-playwright (shared
   Chromium cache), the VS Code mcp.json backup file *.oab-imcp-winpoc.bak.

Report: table "# | check | expected | actual | PASS/FAIL/N/A".
```

## After the run

Paste each report back to the developer session. Each one maps onto `MANUAL-ACCEPTANCE.md`:

| Report | Ticks in MANUAL-ACCEPTANCE.md |
|---|---|
| P0 + P1 (or H1 + P1) | Clean account, from a clone; No VC++ redistributable |
| H1 | SmartScreen / Mark of the Web |
| P3 | Browser for real (`browser_navigate`) |
| P4 | Each real client (first real Gemini run) |
| P5 | Logon autostart of both tasks |
| P6 | uninstall / purge on a clean machine |

To repeat a round, restore the `clean` (or `with-node`) checkpoint on the host.

# Windows POC — manual acceptance (ticket 15)

What a human must verify on a real Windows 11 desktop, beyond what the automated smokes
cover. Spec §8 and appendix C. The POC names (spec §13.2) are pending the author's
confirmation.

## Automated (run these first; no human judgement needed)

From WSL with the Windows cross toolchain (`. windows/xenv.sh`):

```
bash poc/reverse-attach-linux/windows/run-all-smokes.sh
```

Covers: `/mcp` + auth, `sys_info`, `screenshot` (GDI, multi-monitor, mixed DPI, region,
PNG/JPEG), `mouse`/`key` (absolute coords on every monitor, CJK + emoji, combos, no stuck
keys), `powershell` (UTF-8, exit codes, timeout→137, env, cwd, over-long rejected, module
cmdlets), background `exec_*`, reverse-attach real hop + grant persistence + DACL + resume,
Playwright upstream forwarding + offline resilience, switchboard (attach, observe scope,
redial on 1000, stop on 4002), and `install-winpoc.ps1` (install → healthy → token DACL →
uninstall). `exec_start`/GUI-child survival is also confirmed natively via a Scheduled Task.

> Known flake: the CJK-IME exact-typing case in `smoke_windows.sh` passes ~5 of 6 runs on a
> live desktop (the en-US layout switch races the window's first message pump). Re-run; the
> mechanism is correct. Verify it by hand below.

## Manual checklist

- [ ] **CJK / emoji typing** into a real app (Notepad, a browser box) with the zh-TW
      Bopomofo IME active: `key type` produces `繁體中文，標點「。」emoji 🎉👍` exactly, and
      the keyboard layout/IME is restored afterwards.
- [ ] **UIPI against an elevated window**: open an app elevated (Run as administrator, e.g.
      an admin PowerShell). With the node under a normal user, `mouse`/`key` aimed at it
      return a result that warns the input may be blocked (not an unconditional ok), and the
      elevated window does not receive the input.
- [ ] **Lock screen**: lock the desktop (Win+L). `screenshot`, `mouse`, `key` fail with a
      clear "input desktop is not Default / locked" error; nothing is silently reported ok.
- [ ] **UAC prompt**: trigger a UAC consent dialog. The desktop tools fail the same way
      (secure desktop) rather than acting on or screenshotting it.
- [ ] **Restricted execution policy** (clean machine where LocalMachine is Restricted): the
      `powershell` tool still runs a single command (it is not a script); writing and running
      a `.ps1` is blocked as documented.
- [ ] **Smart App Control / SmartScreen**: on a machine with SAC in enforcement, the
      unsigned `oab-imcp-winpoc.exe` is blocked until allowed; record the behaviour. (Signing
      is the author's release decision.)
- [ ] **Microsoft Defender**: confirm the unsigned exe is not quarantined by heuristics, or
      record what happens.
- [ ] **Laptop on battery**: after `install-winpoc.ps1 -Install`, unplug AC — the Scheduled
      Task keeps the node running (ExecutionTimeLimit PT0S, runs on battery).
- [ ] **Logon autostart**: sign out and back in; the task starts the node automatically and
      `/healthz` is green without manual steps.
- [ ] **Tray menu** (`--menu-bar`): the tray icon appears; Copy URL / Copy token put the
      right text on the clipboard; Environment check shows the report; Open log opens the
      agent log; Restart and Quit work.
- [ ] **Real openab-pty hop** (not the mock): lend this Windows node to a real openab-pty
      session and drive `screenshot` / `powershell` from the session CLI.
- [ ] **Real Playwright**: with Node + `@playwright/mcp` installed (`pw-mcp.ps1`), drive
      `browser_navigate` + `browser_snapshot` against a real page.

# OpenAB hands node — Windows (POC)

Lend this Windows computer to a remote agent through OpenAB (Connect / openab-pty /
switchboard), the same way the macOS and Linux nodes work. Names here are POC names,
pending the author's confirmation.

## Install (one step)

From a clone of the repo, double-click `scripts\setup-winpoc.cmd`: it downloads this ZIP from
the newest release, checks its sha256 and runs the installer below for you. Full guide, options
and troubleshooting: [`docs/windows-setup.md`](../../../docs/windows-setup.md).

From this ZIP directly:

1. Unzip anywhere (e.g. your Downloads).
2. **Double-click `install.cmd`.**

That's it — no admin. It installs for your user, starts the node, and sets it to start
again at every sign-in (a Scheduled Task, the Windows equivalent of a macOS LaunchAgent or
a Linux systemd service). It prints your **MCP URL** and **bearer token** when it finishes.

To use Tailscale identity instead of a token, run from a PowerShell prompt:

```powershell
.\install-winpoc.ps1 -Install -AllowLogin you@example.com
```

For remote access, install [Tailscale](https://tailscale.com/download/windows) first; the
installer then runs `tailscale serve` for you and prints an `https://…ts.net/mcp` URL.

## Use it

- **Lend it to an agent**: in OpenAB Connect / Remote, pick this computer. The node dials
  the session and the agent can see the screen, click, type and run PowerShell.
- **Browser tools** (`browser_*`): with [Node.js](https://nodejs.org) installed, the installer
  sets up `@playwright/mcp` 0.0.82 + Chromium as a second logon task (`pw-mcp.ps1`) and has the
  node re-serve it. Installed Node.js later? Run the installer again. `-NoBrowser` skips it.
- **Claude Code**: with `claude` on PATH, the installer registers the node as user-scope MCP
  server `oab-imcp-winpoc` (loopback URL + bearer token; the token is then stored in plain
  text in Claude Code's config). Otherwise it prints the command to paste. `-NoClaudeCode`
  skips it.
- **Other MCP clients** can point at the printed loopback URL with the bearer token.

## Manage

- Status: `.\install-winpoc.ps1 -Status`
- Update: re-run `scripts\setup-winpoc.cmd`, or unzip the new version and double-click
  `install.cmd` again (your token is kept).
- Remove: double-click `uninstall.cmd` (add `-Purge` in a shell to also delete the token,
  saved grants and the browser profile). Uninstall also removes the Playwright task and the
  Claude Code registration.

## Tools

`sys_info`, `screenshot`, `mouse`, `key`, `powershell`, `exec_start`/`exec_poll`/
`exec_list`/`exec_cancel`, and `browser_*` when Playwright is configured. Same contract as
the macOS/Linux nodes.

Unsigned POC build: Windows SmartScreen / Smart App Control may warn or block it until you
allow it. Code signing is the author's release decision.

# OpenAB hands node — Windows (POC)

Lend this Windows computer to a remote agent through OpenAB (Connect / openab-pty /
switchboard), the same way the macOS and Linux nodes work. Names here are POC names,
pending the author's confirmation.

## Install (one step)

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
- **Browser tools** (`browser_*`): install [Node.js](https://nodejs.org), then the node
  picks up `pw-mcp.ps1` (Playwright) automatically.
- **A local MCP client** can also point at the printed loopback URL with the bearer token.

## Manage

- Status: `.\install-winpoc.ps1 -Status`
- Update: unzip the new version, double-click `install.cmd` again (your token is kept).
- Remove: double-click `uninstall.cmd` (add `-Purge` in a shell to also delete the token
  and saved grants).

## Tools

`sys_info`, `screenshot`, `mouse`, `key`, `powershell`, `exec_start`/`exec_poll`/
`exec_list`/`exec_cancel`, and `browser_*` when Playwright is configured. Same contract as
the macOS/Linux nodes.

Unsigned POC build: Windows SmartScreen / Smart App Control may warn or block it until you
allow it. Code signing is the author's release decision.

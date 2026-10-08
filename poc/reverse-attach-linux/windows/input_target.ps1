# Test helper for smoke_windows.sh: a small top-most window that records what reaches it.
#   input_target.ps1 -Out <dir>
# Writes <dir>\target.json once shown: {"hwnd", "left","top","right","bottom" (physical px),
# "box": centre of the text box}. Records ctrl/alt key-downs. When <dir>\done appears it
# writes <dir>\result.json {"text", "keys", "layout_before", "layout_after"} and exits (after 60 s at the latest).
param([Parameter(Mandatory)][string]$Out)
$ErrorActionPreference = 'Stop'
Add-Type -ReferencedAssemblies System.Windows.Forms, System.Drawing -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class Target {
    [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr v);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, IntPtr pid);
    [DllImport("kernel32.dll")] static extern uint GetCurrentThreadId();
    [DllImport("user32.dll")] static extern bool AttachThreadInput(uint a, uint b, bool attach);
    [DllImport("user32.dll")] static extern bool BringWindowToTop(IntPtr h);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
    // Standard workaround for Windows foreground-stealing prevention: attach our input
    // queue to the current foreground thread's, then SetForegroundWindow succeeds.
    public static void ForceForeground(IntPtr h) {
        uint fg = GetWindowThreadProcessId(GetForegroundWindow(), IntPtr.Zero);
        uint me = GetCurrentThreadId();
        AttachThreadInput(me, fg, true);
        BringWindowToTop(h); SetForegroundWindow(h);
        AttachThreadInput(me, fg, false);
    }
}
'@
[void][Target]::SetProcessDpiAwarenessContext([IntPtr]-4)
$form = New-Object Windows.Forms.Form
$form.Text = 'instance-mcp smoke target (closes itself)'; $form.TopMost = $true
$form.StartPosition = 'Manual'; $form.Location = New-Object Drawing.Point 220, 220
$form.Size = New-Object Drawing.Size 640, 200
$tb = New-Object Windows.Forms.TextBox; $tb.Multiline = $true; $tb.Dock = 'Fill'
$tb.Font = New-Object Drawing.Font 'Microsoft JhengHei', 14
$keys = New-Object Collections.Generic.List[string]
$tb.Add_KeyDown({ param($s, $e) if ($e.Control -or $e.Alt) { $keys.Add(("{0}{1}{2}" -f ($(if ($e.Control) {'ctrl+'} else {''})), ($(if ($e.Alt) {'alt+'} else {''})), $e.KeyCode)) } })
$live = Join-Path $Out 'live.txt'
$tb.Add_TextChanged({ try { Set-Content -LiteralPath $live -Value $tb.Text -Encoding UTF8 -NoNewline } catch {} })
$form.Controls.Add($tb); $form.Show(); $form.Activate(); [void]$tb.Focus()
[Target]::ForceForeground($form.Handle); [void]$tb.Focus()
[Windows.Forms.Application]::DoEvents()
$r = New-Object Target+RECT; [void][Target]::GetWindowRect($form.Handle, [ref]$r)
$box = $tb.RectangleToScreen($tb.ClientRectangle)
@{ hwnd = $form.Handle.ToInt64(); left = $r.L; top = $r.T; right = $r.R; bottom = $r.B;
   box = @{ x = [int]($box.X + $box.Width / 2); y = [int]($box.Y + $box.Height / 2) } } |
    ConvertTo-Json -Compress | Set-Content -Encoding UTF8 (Join-Path $Out 'target.json')
$layoutBefore = [Windows.Forms.InputLanguage]::CurrentInputLanguage.Culture.Name
$deadline = (Get-Date).AddSeconds(60)
while (-not (Test-Path (Join-Path $Out 'done')) -and (Get-Date) -lt $deadline) {
    [Windows.Forms.Application]::DoEvents(); Start-Sleep -Milliseconds 20
}
@{ text = $tb.Text; keys = @($keys); layout_before = $layoutBefore;
   layout_after = [Windows.Forms.InputLanguage]::CurrentInputLanguage.Culture.Name } | ConvertTo-Json -Compress |
    Set-Content -Encoding UTF8 (Join-Path $Out 'result.json')
$form.Close()

# Test helper for smoke_windows.sh: prints "<cursorX> <cursorY> <foregroundHwnd> <keysDown>" in
# physical pixels (per-monitor v2) plus how many modifier keys / mouse buttons are down;
# with -SetX X -SetY Y it moves the cursor there first.
param([int]$SetX = [int]::MinValue, [int]$SetY = 0)
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class Probe {
    [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr v);
    [DllImport("user32.dll")] public static extern bool GetCursorPos(out POINT p);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern short GetAsyncKeyState(int vk);
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
}
'@
[void][Probe]::SetProcessDpiAwarenessContext([IntPtr]-4)
if ($SetX -ne [int]::MinValue) { [void][Probe]::SetCursorPos($SetX, $SetY) }
$p = New-Object Probe+POINT; [void][Probe]::GetCursorPos([ref]$p)
$down = @(0x10, 0x11, 0x12, 0x5B, 0x5C, 0x01, 0x02) | Where-Object { ([Probe]::GetAsyncKeyState($_) -band 0x8000) -ne 0 }
"{0} {1} {2} {3}" -f $p.X, $p.Y, [Probe]::GetForegroundWindow().ToInt64(), @($down).Count

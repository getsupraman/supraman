# A PNG of one process's main window, drawn by the window itself (PrintWindow), so another window over
# it does not end up in the picture (the selftest's FOREMAN_SHOTS). -Title picks another of its top-level windows.
param(
    [Parameter(Mandatory)][int]$ProcessId,
    [Parameter(Mandatory)][string]$Out,
    [string]$Title
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class ShotWin32 {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT rect);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr hWnd, IntPtr hdc, uint flags);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr FindWindowEx(IntPtr parent, IntPtr after, string cls, string title);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint pid);
}
'@
[ShotWin32]::SetProcessDPIAware() | Out-Null
$hwnd = (Get-Process -Id $ProcessId).MainWindowHandle
if ($Title) {
    $hwnd = [IntPtr]::Zero
    $w = [IntPtr]::Zero
    while (($w = [ShotWin32]::FindWindowEx([IntPtr]::Zero, $w, [NullString]::Value, $Title)) -ne [IntPtr]::Zero) {
        $owner = 0
        [ShotWin32]::GetWindowThreadProcessId($w, [ref]$owner) | Out-Null
        if ($owner -eq $ProcessId) { $hwnd = $w; break }
    }
}
if ($hwnd -eq [IntPtr]::Zero) { throw "process $ProcessId has no window titled '$Title' (none: its main window)" }
$r = New-Object ShotWin32+RECT
[ShotWin32]::GetWindowRect($hwnd, [ref]$r) | Out-Null
$bmp = New-Object System.Drawing.Bitmap ($r.Right - $r.Left), ($r.Bottom - $r.Top)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$hdc = $g.GetHdc()
# PW_RENDERFULLCONTENT: DirectX content too
$ok = [ShotWin32]::PrintWindow($hwnd, $hdc, 2)
$g.ReleaseHdc($hdc)
$g.Dispose()
if (-not $ok) { throw "PrintWindow failed" }
$bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
$bmp.Dispose()

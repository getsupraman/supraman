# A PNG of the screen area 'x,y,w,h' in physical pixels (the selftest's tray icon on the taskbar, FOREMAN_SHOTS).
param(
    [Parameter(Mandatory)][string]$Out,
    [Parameter(Mandatory)][string]$Screen
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type @'
using System.Runtime.InteropServices;
public static class ShotWin32 {
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
}
'@
[ShotWin32]::SetProcessDPIAware() | Out-Null
$x, $y, $w, $h = $Screen.Split(',') | ForEach-Object { [int]$_ }
$bmp = New-Object System.Drawing.Bitmap $w, $h
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($x, $y, 0, 0, $bmp.Size)
$g.Dispose()
$bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
$bmp.Dispose()

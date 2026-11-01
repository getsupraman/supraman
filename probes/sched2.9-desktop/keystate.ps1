# sched2/9 probe: does a thread on a hidden desktop see the user's held modifier keys?
# Starts a poller on a new hidden desktop (never switched to) and one here; for -Seconds each logs every change of
# GetKeyState (its thread, with a message queue) and GetAsyncKeyState for Shift, Ctrl and Alt. Type or hold a modifier
# meanwhile: a change in the hidden log means the hidden desktop's threads read the real keyboard. Read only: no input
# is sent anywhere.
# -Activate: the hidden poller also shows a window and activates it every 2 s (as each selftest window does when it
# opens); `A` marks an activation in its log.
# Run from the repo root: pwsh -File probes/sched2.9-desktop/keystate.ps1 [-Seconds 60] [-Activate]
param([int]$Seconds = 60, [string]$Poll, [switch]$Activate)

$code = @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class KeyProbe {
    [StructLayout(LayoutKind.Sequential)] public struct MSG { public IntPtr h; public uint m; public IntPtr w; public IntPtr l; public uint t; public int x; public int y; }
    [DllImport("user32.dll")] public static extern short GetKeyState(int vk);
    [DllImport("user32.dll")] public static extern short GetAsyncKeyState(int vk);
    [DllImport("user32.dll")] public static extern bool PeekMessageW(out MSG m, IntPtr h, uint a, uint b, uint r);
    [DllImport("user32.dll", SetLastError = true, CharSet = CharSet.Unicode)] public static extern IntPtr CreateDesktopW(string name, IntPtr dev, IntPtr mode, uint flags, uint access, IntPtr sa);
    [DllImport("user32.dll")] public static extern bool CloseDesktop(IntPtr d);
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)] public struct SI { public int cb; public string r; public string desk; public string title; public int x, y, xs, ys, xc, yc, fill, flags; public short show, r2; public IntPtr r3, i, o, e; }
    [StructLayout(LayoutKind.Sequential)] public struct PI { public IntPtr p, t; public int pid, tid; }
    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)] public static extern bool CreateProcessW(string app, StringBuilder cmd, IntPtr pa, IntPtr ta, bool inherit, uint flags, IntPtr env, string cwd, ref SI si, out PI pi);
    [DllImport("kernel32.dll")] public static extern uint WaitForSingleObject(IntPtr h, uint ms);
    [DllImport("kernel32.dll")] public static extern bool CloseHandle(IntPtr h);
    public static string Sample() {
        MSG m; PeekMessageW(out m, IntPtr.Zero, 0, 0, 0);
        var sb = new StringBuilder();
        foreach (var vk in new[] { 0x10, 0x11, 0x12 }) sb.Append((GetKeyState(vk) < 0 ? "1" : "0") + ((GetAsyncKeyState(vk) & 0x8000) != 0 ? "1" : "0") + " ");
        return sb.ToString().Trim();
    }
}
'@
Add-Type $code

function Poll([string]$out, [int]$seconds, [bool]$activate) {
    $last = ''; $changes = 0
    $until = (Get-Date).AddSeconds($seconds)
    "# columns: Shift Ctrl Alt, each <GetKeyState><GetAsyncKeyState>" | Set-Content $out
    $form = $null; $next = Get-Date
    if ($activate) {
        Add-Type -AssemblyName System.Windows.Forms
        $form = New-Object System.Windows.Forms.Form
        $form.Show()
    }
    while ((Get-Date) -lt $until) {
        if ($form -and (Get-Date) -ge $next) {
            $form.Activate(); [System.Windows.Forms.Application]::DoEvents()
            "{0:HH:mm:ss.fff} A" -f (Get-Date) | Add-Content $out
            $next = (Get-Date).AddSeconds(2)
        }
        $s = [KeyProbe]::Sample()
        if ($s -ne $last) { "{0:HH:mm:ss.fff} {1}" -f (Get-Date), $s | Add-Content $out; $last = $s; $changes++ }
        Start-Sleep -Milliseconds 20
    }
}

if ($Poll) { Poll $Poll $Seconds $Activate.IsPresent; exit 0 }

$scratch = Join-Path $PSScriptRoot ".scratch/keystate-$(Get-Date -Format 'yyyyMMdd-HHmmss')"
New-Item -ItemType Directory -Force $scratch | Out-Null
$hiddenLog = Join-Path $scratch 'hidden.log'; $hereLog = Join-Path $scratch 'here.log'
$name = "supraman-keyprobe-$PID"
# every DESKTOP_* right below DESKTOP_SWITCHDESKTOP, and the standard rights
$desk = [KeyProbe]::CreateDesktopW($name, [IntPtr]::Zero, [IntPtr]::Zero, 0, 0x000F00FF, [IntPtr]::Zero)
if ($desk -eq [IntPtr]::Zero) { throw "CreateDesktop failed: $([Runtime.InteropServices.Marshal]::GetLastWin32Error())" }
$si = New-Object KeyProbe+SI
$si.cb = [Runtime.InteropServices.Marshal]::SizeOf($si); $si.desk = $name
$pi = New-Object KeyProbe+PI
$pwsh = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
$cmd = New-Object Text.StringBuilder "`"$pwsh`" -NoLogo -NoProfile -ExecutionPolicy Bypass -File `"$PSCommandPath`" -Seconds $Seconds -Poll `"$hiddenLog`"$(if ($Activate) { ' -Activate' })"
# CREATE_NO_WINDOW
if (-not [KeyProbe]::CreateProcessW([NullString]::Value, $cmd, [IntPtr]::Zero, [IntPtr]::Zero, $false, 0x08000000, [IntPtr]::Zero, [NullString]::Value, [ref]$si, [ref]$pi)) {
    throw "CreateProcess failed: $([Runtime.InteropServices.Marshal]::GetLastWin32Error())"
}
Poll $hereLog $Seconds $false
[KeyProbe]::WaitForSingleObject($pi.p, 30000) | Out-Null
[KeyProbe]::CloseHandle($pi.p) | Out-Null; [KeyProbe]::CloseHandle($pi.t) | Out-Null
[KeyProbe]::CloseDesktop($desk) | Out-Null
"here ($((Get-Content $hereLog).Count - 1) states):"; Get-Content $hereLog | Select-Object -Skip 1 -First 15
"hidden desktop ($((Get-Content $hiddenLog | Where-Object { $_ -notmatch ' A$' }).Count - 1) states):"; Get-Content $hiddenLog | Where-Object { $_ -notmatch ' A$' } | Select-Object -Skip 1 -First 15

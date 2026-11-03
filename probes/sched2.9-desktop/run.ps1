#Requires -Version 7
# sched2/9 probe: does `supraman.exe --selftest` work on a hidden desktop that is never switched to?
# It runs the selftest on its default (hidden) desktop with SUPRAMAN_SHOTS set, then reports where it ran, whether
# it passed, whether a selftest window ever had the input desktop's focus, and how many window captures hold real
# pixels (a black frame means the hidden desktop is not composed).
# Run from the repo root: pwsh -File probes/sched2.9-desktop/run.ps1 [-Exe <supraman.exe>]
param([string]$Exe = (Join-Path $env:CARGO_TARGET_DIR 'release\supraman.exe'))

$ErrorActionPreference = 'Stop'
$scratch = Join-Path $PSScriptRoot ".scratch/$(Get-Date -Format 'yyyyMMdd-HHmmss')"
$shots = Join-Path $scratch 'shots'
New-Item -ItemType Directory -Force $shots | Out-Null
$log = Join-Path $scratch 'selftest.log'

$env:SUPRAMAN_SHOTS = $shots
Remove-Item Env:SUPRAMAN_SELFTEST_VISIBLE -ErrorAction SilentlyContinue
& $Exe --selftest *> $log
$code = $LASTEXITCODE

Add-Type -AssemblyName System.Drawing
$real = 0; $black = @()
foreach ($png in Get-ChildItem $shots -Filter *.png) {
    $b = [System.Drawing.Bitmap]::FromFile($png.FullName)
    $inked = $false
    for ($x = 0; $x -lt $b.Width -and -not $inked; $x += 9) {
        for ($y = 0; $y -lt $b.Height; $y += 9) {
            if ($b.GetPixel($x, $y).ToArgb() -ne -16777216) { $inked = $true; break }
        }
    }
    if ($inked) { $real++ } else { $black += "$($png.Name) $($b.Width)x$($b.Height)" }
    $b.Dispose()
}

$lines = Get-Content $log
"placement:  $($lines | Select-String '^selftest on' | Select-Object -First 1)"
"result:     exit $code; $(($lines | Select-String '^(PASS|FAIL)' | ForEach-Object Line) -join '; ')"
"focus:      $($lines | Select-String '^(FAIL )?isolation' | Select-Object -First 1)"
"captures:   $real with real pixels, $($black.Count) black$(if ($black) { ': ' + ($black -join ', ') })"
"skipped:    $(($lines | Select-String 'shot: .* (skipped|failed)' | ForEach-Object Line) -join '; ')"
"log:        $log"
exit $code

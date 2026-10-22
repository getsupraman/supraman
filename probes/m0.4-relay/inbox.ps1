#Requires -Version 7
# M0.4 relay inbox watcher, run by the relay session under its Monitor tool.
# Prints the full path of each new *.json file in -Dir, one per line, forever.
# Emitted paths are recorded in <Dir>/.seen so a re-armed watcher does not replay them.
param([Parameter(Mandatory)][string]$Dir)

$ErrorActionPreference = 'Stop'
$Dir = (Resolve-Path $Dir).Path
$seenPath = Join-Path $Dir '.seen'
$seen = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::OrdinalIgnoreCase)
if (Test-Path $seenPath) { Get-Content $seenPath | Where-Object { $_ } | ForEach-Object { [void]$seen.Add($_) } }

while ($true) {
    foreach ($f in Get-ChildItem -Path $Dir -Filter '*.json' -File | Sort-Object Name) {
        if ($seen.Add($f.FullName)) {
            Add-Content -Path $seenPath -Value $f.FullName
            [Console]::Out.WriteLine($f.FullName)
            [Console]::Out.Flush()
        }
    }
    Start-Sleep -Milliseconds 500
}

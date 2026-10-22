# Control probe for m1/7: one fm-selftest-diag-<hex> session that is NEVER attached. Polls its raw
# `claude agents --json --all` row and its process chain for -Seconds, then stops and removes it.
# Answers: does a never-attached session reach state stopped/done on its own?
#   pwsh -File crates/foreman-app/diag/states.ps1 [-Seconds 40]
param([int]$Seconds = 40)
$ErrorActionPreference = 'Stop'
$root = Resolve-Path (Join-Path $PSScriptRoot '..\..\..')
$cwd = Join-Path $root 'target\diag-cwd'
New-Item -ItemType Directory -Force $cwd | Out-Null
$name = 'fm-selftest-diag-{0:x6}' -f (Get-Random -Maximum 0xffffff)
$prompt = 'Reply with the word ready and then wait for input'

function Row {
    (claude agents --json --all | ConvertFrom-Json) | Where-Object { $_.name -eq $name } | Select-Object -First 1
}

function Chain([int]$procId) {
    $parts = @()
    while ($procId -gt 0 -and $parts.Count -lt 8) {
        $p = Get-CimInstance Win32_Process -Filter "ProcessId=$procId"
        if (-not $p) { $parts += "$procId(gone)"; break }
        $exe = ($p.CommandLine -split ' ')[1..2] -join ' '
        $parts += "$procId $($p.Name) [$exe]"
        $procId = $p.ParentProcessId
    }
    $parts -join ' <- '
}

$start = Get-Date
Push-Location $cwd
try {
    $out = claude --bg --name $name --permission-mode auto $prompt
} finally {
    Pop-Location
}
"launched $name : $out"
$last = ''
try {
    while (((Get-Date) - $start).TotalSeconds -lt $Seconds) {
        $row = Row
        $t = [int]((Get-Date) - $start).TotalMilliseconds
        if ($row) {
            $line = "state=$($row.state) status=$($row.status) pid=$($row.pid) waitingFor=$($row.waitingFor)"
            if ($line -ne $last) {
                "+$t ms $line"
                if ($row.pid) { "        chain: $(Chain $row.pid)" }
                $last = $line
            }
        } elseif ($last -ne 'absent') {
            "+$t ms (no row)"
            $last = 'absent'
        }
        Start-Sleep -Milliseconds 500
    }
} finally {
    $row = Row
    if ($row) {
        "cleanup: $(claude stop $row.id)"
        "cleanup: $(claude rm $row.id)"
    }
}

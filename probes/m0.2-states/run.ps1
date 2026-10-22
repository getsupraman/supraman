#Requires -Version 7
# M0.2 probe: launch one throwaway background session and record every distinct state
# `claude agents --json` reports for it, through busy -> permission park -> stop -> rm.
# Run from the repo root: pwsh -File probes/m0.2-states/run.ps1
# Only ever touches the session it launched (name prefix aw-m0-probe-).

$ErrorActionPreference = 'Stop'

$ts = Get-Date -Format 'yyyyMMdd-HHmmss'
$scratch = Join-Path $PSScriptRoot ".scratch/$ts"
New-Item -ItemType Directory -Force $scratch | Out-Null
$logPath = Join-Path $scratch 'states.log'
$rawPath = Join-Path $scratch 'rows.jsonl'

$name = 'aw-m0-probe-' + -join ((1..6) | ForEach-Object { '{0:x}' -f (Get-Random -Maximum 16) })
$prompt = 'Count from 1 to 40 in words, one per line, then use the Bash tool to run New-Item -ItemType File probe.txt'

$t0 = Get-Date
function Log([string]$msg) {
    $line = '[{0,7:0.0}s] {1}' -f ((Get-Date) - $t0).TotalSeconds, $msg
    Write-Host $line
    Add-Content -Path $logPath -Value $line
}

# Plain `--json` drops a session the moment it is stopped; `--all` keeps completed ones until rm.
function Get-ProbeRow([switch]$All) {
    $raw = if ($All) { & claude agents --json --all 2>$null } else { & claude agents --json 2>$null }
    if ($LASTEXITCODE -ne 0 -or -not $raw) { return $null }
    $data = ($raw -join "`n") | ConvertFrom-Json -Depth 20
    $rows = if ($data -is [array]) { $data }
            else {
                $arr = $data.PSObject.Properties | Where-Object { $_.Value -is [array] } | Select-Object -First 1
                if ($arr) { $arr.Value } else { @($data) }
            }
    $rows | Where-Object { $_.name -eq $name } | Select-Object -First 1
}

# Fields whose value changes on every poll would make every tuple distinct; they are recorded
# in rows.jsonl but left out of the transition signature.
$volatile = '(?i)(At|Time|Timestamp|elapsed|duration|uptime|age|ago|tokens|cost|lastActivity)$'

function Get-Signature($row) {
    $parts = foreach ($p in $row.PSObject.Properties) {
        if ($p.Name -match $volatile) { continue }
        $v = if ($null -eq $p.Value) { 'null' }
             elseif ($p.Value -is [string] -or $p.Value -is [ValueType]) { "$($p.Value)" }
             else { $p.Value | ConvertTo-Json -Compress -Depth 10 }
        "$($p.Name)=$v"
    }
    $parts -join ' | '
}

$id = $null
$sawBusy = $false
$sawBlocked = $false
$sawGone = $false
$lastSig = $null

function Poll {
    $row = Get-ProbeRow -All
    $inDefault = if ($null -ne (Get-ProbeRow)) { 'yes' } else { 'no' }
    if ($null -eq $row) {
        $sig = "<absent from --all> | inDefaultList=$inDefault"
    } else {
        Add-Content -Path $rawPath -Value (@{ t = ((Get-Date) - $t0).TotalSeconds; inDefaultList = $inDefault; row = $row } | ConvertTo-Json -Compress -Depth 20)
        $sig = "inDefaultList=$inDefault | " + (Get-Signature $row)
        $text = "$($row.kind) $($row.state) $($row.status)"
        if ($text -match '(?i)busy|working|running|active') { $script:sawBusy = $true }
        if ($text -match '(?i)blocked|waiting|permission' -or $row.waitingFor) { $script:sawBlocked = $true }
        if (-not $script:id) {
            foreach ($k in 'id', 'shortId', 'sessionId') { if ($row.$k) { $script:id = "$($row.$k)"; break } }
        }
    }
    if ($sig -ne $script:lastSig) {
        Log $sig
        $script:lastSig = $sig
    }
    $row
}

$exit = 1
try {
    Log "launch $name in $scratch"
    Push-Location $scratch
    try {
        $out = & claude --bg --name $name --permission-mode default $prompt 2>&1
    } finally {
        Pop-Location
    }
    $outText = ($out | ForEach-Object { "$_" }) -join "`n"
    Log "launch exit=$LASTEXITCODE output: $($outText -replace "`r?`n", ' / ')"
    if ($outText -match '\b([0-9a-f]{6,})\b') { $id = $Matches[1] }
    $printedId = $id
    Log "printed id: $printedId"

    # Phase 1: run until a blocked state is seen, or 180 s.
    $deadline = (Get-Date).AddSeconds(180)
    while ((Get-Date) -lt $deadline -and -not $sawBlocked) {
        Poll | Out-Null
        Start-Sleep -Seconds 1
    }
    # Hold a few seconds so the parked state is recorded more than once.
    1..3 | ForEach-Object { Poll | Out-Null; Start-Sleep -Seconds 1 }

    # Phase 2: stop, observe ~20 s.
    Log "claude stop $id"
    $o = & claude stop $id 2>&1
    Log "stop exit=$LASTEXITCODE output: $((($o | ForEach-Object { "$_" }) -join ' / '))"
    $deadline = (Get-Date).AddSeconds(20)
    while ((Get-Date) -lt $deadline) { Poll | Out-Null; Start-Sleep -Seconds 1 }

    # Phase 3: rm, poll until the row is gone (60 s).
    Log "claude rm $id"
    $o = & claude rm $id 2>&1
    Log "rm exit=$LASTEXITCODE output: $((($o | ForEach-Object { "$_" }) -join ' / '))"
    $deadline = (Get-Date).AddSeconds(60)
    while ((Get-Date) -lt $deadline) {
        if ($null -eq (Poll)) { $sawGone = $true; break }
        Start-Sleep -Seconds 1
    }

    $missing = @()
    if (-not $sawBusy) { $missing += 'busy/working state' }
    if (-not $sawBlocked) { $missing += 'blocked state or waitingFor' }
    if (-not $sawGone) { $missing += 'row leaving the list after rm' }
    if ($missing.Count -eq 0) {
        $exit = 0
        Log 'RESULT PASS: saw busy, blocked, and row gone after rm'
    } else {
        Log "RESULT FAIL: missing $($missing -join ', ')"
    }
} finally {
    if ($id -and $null -ne (Get-ProbeRow -All)) {
        Log "cleanup: stop + rm $id"
        & claude stop $id 2>&1 | Out-Null
        & claude rm $id 2>&1 | Out-Null
    }
}
exit $exit

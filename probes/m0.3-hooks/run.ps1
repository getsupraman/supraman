#Requires -Version 7
# M0.3 probe: do Notification, SessionStart(compact) and Stop hooks fire for a `claude --bg` session,
# and does a SessionStart hook's stdout re-inject context after a compact?
# Run from the repo root: pwsh -File probes/m0.3-hooks/run.ps1
# Only ever touches sessions it launched (name prefix aw-m0-hooks-).

$ErrorActionPreference = 'Stop'

$ts = Get-Date -Format 'yyyyMMdd-HHmmss'
$scratch = Join-Path $PSScriptRoot ".scratch/$ts"
New-Item -ItemType Directory -Force (Join-Path $scratch '.claude') | Out-Null
$scratch = (Resolve-Path $scratch).Path
$logPath = Join-Path $scratch 'run.log'
$hooksPath = Join-Path $scratch 'hooks.jsonl'
$hookScript = (Resolve-Path (Join-Path $PSScriptRoot 'hook.ps1')).Path
$pwsh = (Get-Command pwsh).Source

$t0 = Get-Date
function Log([string]$msg) {
    $line = '[{0,7:0.0}s] {1}' -f ((Get-Date) - $t0).TotalSeconds, $msg
    Write-Host $line
    Add-Content -Path $logPath -Value $line
}

# Exec form (command + args): no shell, so Git Bash vs PowerShell selection does not matter.
$hooks = [ordered]@{}
foreach ($ev in 'Notification', 'SessionStart', 'Stop', 'PreCompact', 'UserPromptSubmit') {
    $hooks[$ev] = @(@{
        hooks = @(@{
            type    = 'command'
            command = $pwsh
            args    = @('-NoProfile', '-NonInteractive', '-File', $hookScript, '-Event', $ev, '-Out', $hooksPath)
            timeout = 30
        })
    })
}
$settingsPath = Join-Path $scratch '.claude/settings.json'
@{ hooks = $hooks } | ConvertTo-Json -Depth 10 | Set-Content -Path $settingsPath -Encoding utf8NoBOM

function Get-Events {
    if (-not (Test-Path $hooksPath)) { return @() }
    @(Get-Content $hooksPath | Where-Object { $_ } | ForEach-Object { $_ | ConvertFrom-Json -Depth 50 })
}

function Get-ProbeRow([string]$Name) {
    $raw = & claude agents --json --all 2>$null
    if ($LASTEXITCODE -ne 0 -or -not $raw) { return $null }
    $data = ($raw -join "`n") | ConvertFrom-Json -Depth 20
    $rows = if ($data -is [array]) { $data }
            else {
                $arr = $data.PSObject.Properties | Where-Object { $_.Value -is [array] } | Select-Object -First 1
                if ($arr) { $arr.Value } else { @($data) }
            }
    $rows | Where-Object { $_.name -eq $Name } | Select-Object -First 1
}

# `claude --resume <id> -p` from the scratch dir so the same project-local settings apply.
function Invoke-Resume([string]$SessionId, [string]$Prompt) {
    Push-Location $scratch
    try { $o = & claude --resume $SessionId -p $Prompt 2>&1 } finally { Pop-Location }
    $text = ($o | ForEach-Object { "$_" }) -join "`n"
    Log "resume -p '$Prompt' exit=$LASTEXITCODE output: $($text -replace "`r?`n", ' / ')"
    $text
}

$launched = [System.Collections.Generic.List[string]]::new()
$exit = 1
try {
    $name = 'aw-m0-hooks-' + -join ((1..6) | ForEach-Object { '{0:x}' -f (Get-Random -Maximum 16) })
    $prompt = 'Use the Bash tool to run New-Item -ItemType File probe.txt and then say done'
    Log "launch $name in $scratch"
    Push-Location $scratch
    try { $out = & claude --bg --name $name --permission-mode default $prompt 2>&1 } finally { Pop-Location }
    $launched.Add($name)
    Log "launch exit=$LASTEXITCODE output: $((($out | ForEach-Object { "$_" }) -join ' / '))"

    # Phase 1: wait for the permission park (agents row blocked) and the Notification hook, 180 s.
    $row = $null; $blocked = $false
    $deadline = (Get-Date).AddSeconds(180)
    while ((Get-Date) -lt $deadline) {
        $row = Get-ProbeRow $name
        if ($row -and $row.state -eq 'blocked') { $blocked = $true }
        if ($blocked -and (Get-Events | Where-Object event -eq 'Notification')) { break }
        Start-Sleep -Seconds 1
    }
    if (-not $row) { throw "no agents row for $name" }
    $sid = "$($row.sessionId)"
    Log "row state=$($row.state) waitingFor=$($row.waitingFor) sessionId=$sid blocked=$blocked"
    Log "events so far: $((Get-Events | ForEach-Object event) -join ', ')"

    # Phase 2: nobody can click Yes unattended -> stop it.
    $o = & claude stop $row.id 2>&1
    Log "stop exit=$LASTEXITCODE output: $((($o | ForEach-Object { "$_" }) -join ' / '))"
    Start-Sleep -Seconds 3

    # Phase 3: compact non-interactively, then ask for the nonce.
    Invoke-Resume $sid '/compact' | Out-Null
    $compactStart = Get-Events | Where-Object { $_.event -eq 'SessionStart' -and $_.payload.source -eq 'compact' } | Select-Object -Last 1
    $reply = Invoke-Resume $sid 'List every SUPRAMAN-NONCE value present in your context, each with the SessionStart source it says it came from. Reply with text only, use no tools.'

    $events = Get-Events
    $counts = $events | Group-Object event | ForEach-Object { "$($_.Name)=$($_.Count)" }
    Log "hook events: $($counts -join ', ')"
    foreach ($e in $events) {
        $fields = ($e.payload.PSObject.Properties.Name | Sort-Object) -join ','
        $extra = if ($e.payload.source) { " source=$($e.payload.source)" } elseif ($e.payload.notification_type) { " type=$($e.payload.notification_type)" } elseif ($e.payload.trigger) { " trigger=$($e.payload.trigger)" } else { '' }
        Log "  $($e.ts) $($e.event)$extra nonce=$($e.nonce) fields=[$fields]"
    }

    $missing = @()
    if (-not ($events | Where-Object event -eq 'Notification')) { $missing += 'Notification hook' }
    if (-not $compactStart) { $missing += 'SessionStart with source=compact' }
    if (-not ($events | Where-Object event -eq 'Stop')) { $missing += 'Stop hook' }
    if ($compactStart) {
        $hit = $reply -match [regex]::Escape($compactStart.nonce)
        Log "compact-time nonce $($compactStart.nonce) in post-compact reply: $hit"
        if (-not $hit) { $missing += "compact-time nonce $($compactStart.nonce) in post-compact reply" }
    }
    if ($missing.Count -eq 0) {
        $exit = 0
        Log 'RESULT PASS: Notification, SessionStart(compact), Stop fired; post-compact reply carries the compact-time nonce'
    } else {
        Log "RESULT FAIL: missing $($missing -join '; ')"
    }
} catch {
    Log "ERROR: $_"
} finally {
    foreach ($n in $launched) {
        $r = Get-ProbeRow $n
        if ($r) {
            & claude stop $r.id 2>&1 | Out-Null
            $o = & claude rm $r.id 2>&1
            Log "cleanup $n ($($r.id)): rm exit=$LASTEXITCODE $((($o | ForEach-Object { "$_" }) -join ' / '))"
        }
    }
}
exit $exit

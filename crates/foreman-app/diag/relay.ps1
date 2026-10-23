#Requires -Version 7
# m3/1: the relay as Foreman launches it (plugins/foreman/roles/relay.md, `fm inbox` under Monitor, `--add-dir`
# for its outbox), driven with `fm send` / read back from the outbox. Per round: drop -> target transcript
# enqueue -> outbox file seen. -HoldMinutes N keeps the relay up N minutes after round 1 and runs round 2
# then (Monitor expires after 30 min: re-arm). Every ready.json rewrite (= an arm) and every change of the
# relay's row (state/status/waitingFor/pid: a park shows here) is logged.
# Sessions: fm-selftest-<hex>-relay and fm-selftest-<hex>-target, cwd target\selftest-<hex>-relaydiag,
# FOREMAN_HOME target\relay-diag-home. Both stopped and removed at the end.
#   pwsh -File crates/foreman-app/diag/relay.ps1 [-TargetMode auto|default|acceptEdits|plan] [-HoldMinutes 32]
param([string]$TargetMode = 'auto', [int]$HoldMinutes = 0, [int]$TimeoutSec = 180)
$ErrorActionPreference = 'Stop'

$root = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path
$hex = '{0:x6}' -f (Get-Random -Maximum 0xffffff)
$workspace = "selftest-$hex"
$home_ = Join-Path $root 'target\relay-diag-home'
$cwd = Join-Path $root "target\selftest-$hex-relaydiag"
$relayDir = Join-Path $home_ "relay\$workspace"
$outbox = Join-Path $relayDir 'outbox'
$plugin = Join-Path $root 'plugins\foreman'
$fm = Join-Path $root 'target\release\fm.exe'
$relayName = "fm-$workspace-relay"
$targetName = "fm-$workspace-target"
New-Item -ItemType Directory -Force $cwd, (Join-Path $relayDir 'inbox'), $outbox, (Join-Path $home_ 'roles') | Out-Null
$logPath = Join-Path $home_ "relay-$hex.log"
function Fwd([string]$p) { $p -replace '\\', '/' }

$t0 = Get-Date
function Log([string]$msg) {
    $line = '[{0:HH:mm:ss.fff} +{1,7:0.0}s] {2}' -f (Get-Date), ((Get-Date) - $t0).TotalSeconds, $msg
    Write-Host $line
    Add-Content -Path $logPath -Value $line
}

function Rows { @(claude agents --json --all | ConvertFrom-Json) }
$lastSig = @{}
function Watch-Rows {
    $rows = Rows
    $found = @{}
    foreach ($n in @($relayName, $targetName)) {
        $r = $rows | Where-Object { $_.name -eq $n } | Select-Object -First 1
        if (-not $r) { continue }
        $found[$n] = $r
        $sig = "state=$($r.state) status=$($r.status) waitingFor=$($r.waitingFor) pid=$($r.pid)"
        if ($lastSig[$n] -ne $sig) { Log "row $n $sig"; $lastSig[$n] = $sig }
    }
    $found
}
$lastReady = $null
function Watch-Ready {
    $f = Join-Path $outbox 'ready.json'
    if (Test-Path $f) {
        $t = (Get-Item $f).LastWriteTime
        if ($t -ne $script:lastReady) { Log "ready.json written $($t.ToString('HH:mm:ss.fff'))"; $script:lastReady = $t }
    }
}
function Find-Transcript($row) {
    $projects = Join-Path $HOME '.claude/projects'
    Get-ChildItem -Path $projects -Filter "$($row.sessionId).jsonl" -File -Recurse -Depth 1 -ErrorAction SilentlyContinue | Select-Object -First 1 -ExpandProperty FullName
}
function First-Line([string]$path, [string]$needle) {
    if (-not $path -or -not (Test-Path $path)) { return $null }
    foreach ($line in Get-Content $path) {
        if ($line -notlike "*$needle*") { continue }
        $stamp = [regex]::Match($line, '"timestamp":"([^"]+)"').Groups[1].Value
        if ($stamp) { return [datetimeoffset]::Parse($stamp, [cultureinfo]::InvariantCulture) }
    }
    $null
}

$env:FOREMAN_HOME = $home_
try {
    & $fm plugin install $plugin | Out-Null
    $template = (Get-Content (Join-Path $plugin 'roles\relay.md') -Raw).Replace('{fm}', (Fwd (Join-Path $plugin 'bin\fm.exe'))).Replace('{home}', (Fwd $home_))
    Set-Content -Path (Join-Path $home_ 'roles\relay.md') -Value $template -Encoding utf8NoBOM
    $prompt = $template.Replace('{workspace}', $workspace).Trim()
    $settings = @{ env = @{ FOREMAN_HOME = $home_; FOREMAN_ROLE = 'relay'; FOREMAN_WORKSPACE = $workspace }; worktree = @{ bgIsolation = 'none' } } | ConvertTo-Json -Compress
    $tsettings = '{"worktree":{"bgIsolation":"none"}}'
    $targetPrompt = 'This is an automated Foreman selftest. When another session sends you a message, reply to its sender with SendMessage repeating the FOREMAN-RELAY nonce it contains, then wait for more. Do nothing else.'
    Log "workspace $workspace home $home_ cwd $cwd target mode $TargetMode hold $HoldMinutes min"
    Push-Location $cwd
    try {
        $o = claude --bg --name $relayName --permission-mode auto --strict-mcp-config --plugin-dir $plugin --add-dir $relayDir --settings $settings -- $prompt 2>&1
        Log "launch relay: $($o -join ' | ')"
        $o = claude --bg --name $targetName --permission-mode $TargetMode --strict-mcp-config --settings $tsettings -- $targetPrompt 2>&1
        Log "launch target: $($o -join ' | ')"
    } finally { Pop-Location }

    $deadline = (Get-Date).AddSeconds($TimeoutSec)
    do { Start-Sleep -Milliseconds 300; $rows = Watch-Rows; Watch-Ready } until (($script:lastReady -and $rows.Count -eq 2) -or (Get-Date) -gt $deadline)
    if (-not $script:lastReady) { throw 'relay never wrote ready.json' }
    Log "relay armed after $([math]::Round(((Get-Date) - $t0).TotalSeconds, 1)) s"
    # let both first turns end
    $settle = (Get-Date).AddSeconds(15)
    while ((Get-Date) -lt $settle) { $rows = Watch-Rows; Watch-Ready; Start-Sleep -Milliseconds 500 }

    $rounds = if ($HoldMinutes -gt 0) { 2 } else { 3 }
    for ($round = 1; $round -le $rounds; $round++) {
        if ($round -eq 2 -and $HoldMinutes -gt 0) {
            Log "holding $HoldMinutes min"
            $until = (Get-Date).AddMinutes($HoldMinutes)
            while ((Get-Date) -lt $until) { $rows = Watch-Rows; Watch-Ready; Start-Sleep -Seconds 5 }
        }
        $nonce = "FOREMAN-RELAY-{0:x8}" -f (Get-Random)
        $before = @(Get-ChildItem $outbox -Filter 'msg-*.json' -ErrorAction SilentlyContinue | ForEach-Object Name)
        $drop = Get-Date
        $o = & $fm send $workspace $targetName "$nonce ping (round $round)`nsecond line" 2>&1
        Log "round $round dropped: $($o -join ' | ')"
        $hit = $null
        $deadline = $drop.AddSeconds($TimeoutSec)
        while (-not $hit -and (Get-Date) -lt $deadline) {
            foreach ($f in Get-ChildItem $outbox -Filter 'msg-*.json' -ErrorAction SilentlyContinue) {
                if ($before -contains $f.Name) { continue }
                $text = Get-Content $f.FullName -Raw -ErrorAction SilentlyContinue
                if ($text -like "*$nonce*") { $hit = $f; $hitAt = Get-Date; break }
                if ($text) { Log "outbox $($f.Name): $text"; $before += $f.Name }
            }
            if (-not $hit) { $rows = Watch-Rows; Watch-Ready; Start-Sleep -Milliseconds 100 }
        }
        $rows = Watch-Rows
        $tTarget = First-Line (Find-Transcript $rows[$targetName]) $nonce
        $tRelay = First-Line (Find-Transcript $rows[$relayName]) $nonce
        $d = [datetimeoffset]$drop
        $fmt = { param($t) if ($t) { '{0:0.00} s' -f ($t - $d).TotalSeconds } else { '-' } }
        if ($hit) {
            Log ("round {0} PASS: app->relay transcript {1}, app->target {2}, round trip {3:0.00} s ({4}: {5})" -f $round, (& $fmt $tRelay), (& $fmt $tTarget), ($hitAt - $drop).TotalSeconds, $hit.Name, (Get-Content $hit.FullName -Raw))
        } else {
            Log ("round {0} FAIL: no outbox message with the nonce within {1} s; app->relay {2}, app->target {3}" -f $round, $TimeoutSec, (& $fmt $tRelay), (& $fmt $tTarget))
        }
        & $fm messages $workspace | Select-Object -Last 3 | ForEach-Object { Log "fm messages: $_" }
    }
    Log "relay transcript: $(Find-Transcript $rows[$relayName])"
} catch {
    Log "ERROR: $_"
} finally {
    foreach ($n in @($targetName, $relayName)) {
        foreach ($r in @(Rows | Where-Object { $_.name -eq $n })) {
            if ($r.pid) { Log "cleanup stop ${n}: $(claude stop $r.id 2>&1)" }
        }
    }
    Start-Sleep -Seconds 3
    foreach ($n in @($targetName, $relayName)) {
        foreach ($r in @(Rows | Where-Object { $_.name -eq $n })) { Log "cleanup rm ${n}: $(claude rm $r.id 2>&1)" }
    }
    $stray = @(Get-CimInstance Win32_Process -Filter "Name = 'fm.exe'" | Where-Object { $_.CommandLine -like "*$workspace*" })
    Log "fm inbox watchers left: $($stray.Count)"
    Remove-Item -Recurse -Force $cwd -ErrorAction SilentlyContinue
    Log "log: $logPath"
}

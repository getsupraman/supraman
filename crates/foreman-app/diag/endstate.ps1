#Requires -Version 7
# m3/3: which end of a turn leaves a --bg row at `state=blocked status=idle` (m1/7's unexplained state) instead of
# `done`? m2/4: the daemon respawns a killed `blocked` session itself, so the supervisor's resume never comes.
# Sessions (cwd target\selftest-<hex>-endstate, --strict-mcp-config, bgIsolation none, auto), each one turn:
#   wait   replies "I'm now waiting for the next message and won't check for one in the meantime."
#   ready  replies the single word "ready"
#   ask    replies "Should I continue?"
#   send   SendMessage "hi" to fm-selftest-<hex>-sink, then replies "ready"
#   sink   told to do nothing when a message arrives but reply "ok"
#   word   replies the single word "waiting"
#   tsrch  ToolSearch select:SendMessage, then "ready"
#   skill  Skill foreman:repo-controller (read, not followed), then "ready"
# Every session gets --plugin-dir plugins/foreman (no FOREMAN_HOME: its hooks do nothing). Logs each row's
# state/status change for -Seconds, then stops and removes them.
#   pwsh -File crates/foreman-app/diag/endstate.ps1 [-Seconds 90] [-Only wait,ready,ask,send,word,tsrch,skill]
param([int]$Seconds = 90, [string[]]$Only = @())
$ErrorActionPreference = 'Stop'

$root = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path
$hex = '{0:x6}' -f (Get-Random -Maximum 0xffffff)
$cwd = Join-Path $root "target\selftest-$hex-endstate"
New-Item -ItemType Directory -Force $cwd | Out-Null
$t0 = Get-Date
function Log([string]$msg) { Write-Host ('[{0:HH:mm:ss.fff} +{1,6:0.0}s] {2}' -f (Get-Date), ((Get-Date) - $t0).TotalSeconds, $msg) }
function Rows { @(claude agents --json --all | ConvertFrom-Json) }

$p = 'This is an automated Foreman probe. '
$names = [ordered]@{
    sink  = @("fm-selftest-$hex-sink", "${p}When a message arrives, reply with the single word ok and nothing else. Now reply with the single word ok.")
    wait  = @("fm-selftest-$hex-wait", "${p}Reply with exactly this sentence and nothing else: I'm now waiting for the next message and won't check for one in the meantime.")
    ready = @("fm-selftest-$hex-ready", "${p}Reply with the single word ready and nothing else.")
    ask   = @("fm-selftest-$hex-ask", "${p}Reply with exactly this question and nothing else: Should I continue?")
    send  = @("fm-selftest-$hex-send", "${p}Use the SendMessage tool to send the text hi to the session fm-selftest-$hex-sink, then reply with the single word ready and nothing else.")
    word  = @("fm-selftest-$hex-word", "${p}Reply with the single word waiting and nothing else.")
    tsrch = @("fm-selftest-$hex-tsrch", "${p}Use the ToolSearch tool with the query select:SendMessage, then reply with the single word ready and nothing else.")
    skill = @("fm-selftest-$hex-skill", "${p}Invoke the skill foreman:repo-controller with the Skill tool, read it but do not follow it, then reply with the single word ready and nothing else.")
}
$Only = @($Only | ForEach-Object { $_ -split ',' })
if ($Only) { foreach ($k in @($names.Keys)) { if ($k -ne 'sink' -and $Only -notcontains $k) { $names.Remove($k) } } }
$plugin = Join-Path $root 'plugins\foreman'
$settings = '{"worktree":{"bgIsolation":"none"}}'
try {
    Push-Location $cwd
    try {
        foreach ($k in $names.Keys) {
            Log "launch $k : $((claude --bg --name $names[$k][0] --permission-mode auto --strict-mcp-config --plugin-dir $plugin --settings $settings -- $names[$k][1] 2>&1 | Select-Object -First 1))"
            if ($k -eq 'sink') { Start-Sleep -Seconds 8 }
        }
    } finally { Pop-Location }
    $last = @{}
    $until = (Get-Date).AddSeconds($Seconds)
    while ((Get-Date) -lt $until) {
        $rows = Rows
        foreach ($k in $names.Keys) {
            $r = $rows | Where-Object { $_.name -eq $names[$k][0] } | Select-Object -First 1
            if (-not $r) { continue }
            $sig = "state=$($r.state) status=$($r.status) waitingFor=$($r.waitingFor) pid=$($r.pid)"
            if ($last[$k] -ne $sig) { Log "row $k $sig"; $last[$k] = $sig }
        }
        Start-Sleep -Milliseconds 500
    }
    foreach ($k in $names.Keys) { Log "final $k $($last[$k])" }
} finally {
    foreach ($r in @(Rows | Where-Object { $_.name -like "fm-selftest-$hex-*" })) { if ($r.pid) { claude stop $r.id | Out-Null } }
    Start-Sleep -Seconds 3
    foreach ($r in @(Rows | Where-Object { $_.name -like "fm-selftest-$hex-*" })) { Log "rm $($r.name): $(claude rm $r.id 2>&1)" }
    Log "left: $(@(Rows | Where-Object { $_.name -like "fm-selftest-$hex-*" }).Count)"
    Remove-Item -Recurse -Force $cwd -ErrorAction SilentlyContinue
}

# m2/4: what does `claude agents --json --all` show after a --bg session's process is killed?
# Launches one fm-selftest-kill-<hex> session (cwd target\diag-kill, bgIsolation none), waits for its pid,
# kills exactly that pid (taskkill /PID, no /T), polls the raw row for -Seconds, then stops and removes it.
#   pwsh -File crates/foreman-app/diag/kill.ps1 [-Seconds 40] [-Age 0] [-Prompt <text>]   (-Age: seconds to wait after the pid shows)
param([int]$Seconds = 40, [int]$Age = 0, [string]$Prompt = 'Reply with the word ready and then wait for input')
$ErrorActionPreference = 'Stop'
$root = Resolve-Path (Join-Path $PSScriptRoot '..\..\..')
$cwd = Join-Path $root 'target\diag-kill'
New-Item -ItemType Directory -Force $cwd | Out-Null
$name = 'fm-selftest-kill-{0:x6}' -f (Get-Random -Maximum 0xffffff)
$prompt = $Prompt
$settings = '{"worktree":{"bgIsolation":"none"}}'

function Row {
    (claude agents --json --all | ConvertFrom-Json) | Where-Object { $_.name -eq $name } | Select-Object -First 1
}

Push-Location $cwd
try {
    $out = claude --bg --name $name --permission-mode auto --strict-mcp-config --settings $settings -- $prompt
} finally {
    Pop-Location
}
"launched $name : $out"
try {
    $deadline = (Get-Date).AddSeconds(30)
    do { Start-Sleep -Milliseconds 500; $row = Row } until (($row -and $row.pid) -or (Get-Date) -gt $deadline)
    if (-not $row.pid) { throw "no pid within 30 s" }
    if ($Age -gt 0) { Start-Sleep -Seconds $Age; $row = Row }
    $victim = [int]$row.pid
    "before kill: id=$($row.id) sid=$($row.sessionId) state=$($row.state) status=$($row.status) pid=$victim"
    $p = Get-CimInstance Win32_Process -Filter "ProcessId=$victim"
    "victim: $($p.Name) $(($p.CommandLine -split ' ')[1..2] -join ' ')"
    $killed = Get-Date
    taskkill /PID $victim /F
    $last = ''
    while (((Get-Date) - $killed).TotalSeconds -lt $Seconds) {
        $row = Row
        $t = [int]((Get-Date) - $killed).TotalMilliseconds
        $alive = [bool](Get-Process -Id $victim -ErrorAction SilentlyContinue)
        $line = if ($row) { "state=$($row.state) status=$($row.status) pid=$($row.pid) victimAlive=$alive" } else { 'no row' }
        if ($line -ne $last) { "+$t ms $line"; $last = $line }
        Start-Sleep -Milliseconds 500
    }
} finally {
    $row = Row
    if ($row) {
        "cleanup: $(claude stop $row.id)"
        Start-Sleep -Seconds 2
        "cleanup: $(claude rm $row.id)"
    }
    Remove-Item -Recurse -Force $cwd -ErrorAction SilentlyContinue
}

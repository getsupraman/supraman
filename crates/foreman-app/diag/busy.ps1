#Requires -Version 7
# m3/2: which signal says "mid-turn" for a session that waits for cross-session messages (m3/1 saw such an
# idle target keep state=working and hold a busy-budget slot). One session fm-selftest-<hex>-busy, launched
# like the relay's selftest target (auto, told to answer messages, then wait) plus plugins/foreman for hook
# events (FOREMAN_HOME target\busy-diag-home). Logs every change of its row (state/status/pid) with its last
# hook event for -Seconds; send it messages from another session meanwhile. Then stops and removes it.
#   pwsh -File crates/foreman-app/diag/busy.ps1 [-Seconds 300]
param([int]$Seconds = 300)
$ErrorActionPreference = 'Stop'

$root = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path
$hex = '{0:x6}' -f (Get-Random -Maximum 0xffffff)
$home_ = Join-Path $root 'target\busy-diag-home'
$cwd = Join-Path $root "target\selftest-$hex-busy"
$plugin = Join-Path $root 'plugins\foreman'
$fm = Join-Path $root 'target\release\fm.exe'
$name = "fm-selftest-$hex-busy"
New-Item -ItemType Directory -Force $cwd, (Join-Path $home_ 'roles') | Out-Null
$logPath = Join-Path $home_ "busy-$hex.log"

$t0 = Get-Date
function Log([string]$msg) {
    $line = '[{0:HH:mm:ss.fff} +{1,7:0.0}s] {2}' -f (Get-Date), ((Get-Date) - $t0).TotalSeconds, $msg
    Write-Host $line
    Add-Content -Path $logPath -Value $line
}
function Rows { @(claude agents --json --all | ConvertFrom-Json) }
function Events([string]$sid) {
    $f = Join-Path $home_ "events\$sid.jsonl"
    if (-not (Test-Path $f)) { return @() }
    @(Get-Content $f | ForEach-Object { $e = $_ | ConvertFrom-Json; "$($e.event)$(if ($e.source) { "($($e.source))" })$(if ($e.notification_type) { "($($e.notification_type))" })" })
}

$env:FOREMAN_HOME = $home_
try {
    Log "install: $(& $fm plugin install $plugin)"
    $settings = @{ env = @{ FOREMAN_HOME = $home_; FOREMAN_ROLE = 'probe'; FOREMAN_WORKSPACE = "selftest-$hex" }; worktree = @{ bgIsolation = 'none' } } | ConvertTo-Json -Compress
    $prompt = 'This is an automated Foreman selftest. When another session sends you a message, reply to its sender with SendMessage repeating the FOREMAN-RELAY nonce it contains, then wait for more. Do nothing else.'
    Push-Location $cwd
    try {
        Log "launch: $((claude --bg --name $name --permission-mode auto --strict-mcp-config --plugin-dir $plugin --settings $settings -- $prompt 2>&1) -join ' | ')"
    } finally { Pop-Location }
    $last = ''
    $seen = 0
    $until = (Get-Date).AddSeconds($Seconds)
    while ((Get-Date) -lt $until) {
        $r = Rows | Where-Object { $_.name -eq $name } | Select-Object -First 1
        if ($r) {
            $events = Events $r.sessionId
            if ($events.Count -gt $seen) { Log "hook: $($events[$seen..($events.Count - 1)] -join ', ')"; $seen = $events.Count }
            $sig = "state=$($r.state) status=$($r.status) waitingFor=$($r.waitingFor) pid=$($r.pid)"
            if ($sig -ne $last) { Log "row $sig"; $last = $sig }
        }
        Start-Sleep -Milliseconds 500
    }
} catch {
    Log "ERROR: $_"
} finally {
    foreach ($r in @(Rows | Where-Object { $_.name -eq $name })) { if ($r.pid) { Log "cleanup stop: $(claude stop $r.id 2>&1)" } }
    Start-Sleep -Seconds 3
    foreach ($r in @(Rows | Where-Object { $_.name -eq $name })) { Log "cleanup rm: $(claude rm $r.id 2>&1)" }
    Log "left: $(@(Rows | Where-Object { $_.name -eq $name }).Count)"
    Remove-Item -Recurse -Force $cwd -ErrorAction SilentlyContinue
    Log "log: $logPath"
}

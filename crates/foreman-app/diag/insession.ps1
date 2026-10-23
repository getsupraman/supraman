#Requires -Version 7
# m3/2: what a session Foreman launches can run, and what its row says between turns.
# Sessions (cwd target\selftest-<hex>-insession, FOREMAN_HOME target\insession-diag-home, plugins/foreman with
# this build's fm.exe installed, --strict-mcp-config, bgIsolation none):
#   fm-selftest-<hex>-path    auto: runs `where.exe fm` and `fm --version` (is the plugin's bin/ on PATH?)
#   fm-selftest-<hex>-allow   default mode + --allowedTools "Bash(<abs fm>:*)": runs `<abs fm> --version`
#   fm-selftest-<hex>-bare    default mode + --allowedTools "Bash(fm:*)": runs `fm --version`
#   fm-selftest-<hex>-sid     --session-id <uuid>: does the row carry that sessionId?
#   fm-selftest-<hex>-ps      default mode + --allowedTools "PowerShell(fm:*)": runs `fm --version` with PowerShell
#   fm-selftest-<hex>-ps2     the same with "PowerShell(fm *)"
#   fm-selftest-<hex>-none    default mode, no rule: runs `fm --version` with Bash (the control: must park)
# A parked one (default mode, rule did not match) shows as state=blocked/waitingFor. Every row change of every
# session is logged with the last hook event of that session (busy signal), for -Seconds; then all are stopped
# and removed.
#   pwsh -File crates/foreman-app/diag/insession.ps1 [-Seconds 150] [-Only path,allow,bare,sid,ps,ps2,none]
param([int]$Seconds = 150, [string[]]$Only = @('path', 'allow', 'bare', 'sid', 'ps', 'ps2', 'none'))
$ErrorActionPreference = 'Stop'

$root = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path
$hex = '{0:x6}' -f (Get-Random -Maximum 0xffffff)
$home_ = Join-Path $root 'target\insession-diag-home'
$cwd = Join-Path $root "target\selftest-$hex-insession"
$plugin = Join-Path $root 'plugins\foreman'
$fm = Join-Path $root 'target\release\fm.exe'
function Fwd([string]$p) { $p -replace '\\', '/' }
$fmPlugin = Fwd (Join-Path $plugin 'bin\fm.exe')
New-Item -ItemType Directory -Force $cwd, (Join-Path $home_ 'roles') | Out-Null
$logPath = Join-Path $home_ "insession-$hex.log"

$t0 = Get-Date
function Log([string]$msg) {
    $line = '[{0:HH:mm:ss.fff} +{1,7:0.0}s] {2}' -f (Get-Date), ((Get-Date) - $t0).TotalSeconds, $msg
    Write-Host $line
    Add-Content -Path $logPath -Value $line
}
function Rows { @(claude agents --json --all | ConvertFrom-Json) }
function LastEvent([string]$sid) {
    $f = Join-Path $home_ "events\$sid.jsonl"
    if (-not (Test-Path $f)) { return '-' }
    $last = Get-Content $f | Select-Object -Last 1
    if (-not $last) { return '-' }
    $e = $last | ConvertFrom-Json
    "$($e.event)$(if ($e.source) { "($($e.source))" })"
}
function Find-Transcript([string]$sid) {
    Get-ChildItem -Path (Join-Path $HOME '.claude/projects') -Filter "$sid.jsonl" -File -Recurse -Depth 1 -ErrorAction SilentlyContinue | Select-Object -First 1 -ExpandProperty FullName
}
function LastText([string]$sid) {
    $t = Find-Transcript $sid
    if (-not $t) { return '' }
    $text = ''
    foreach ($line in Get-Content $t) {
        $o = $line | ConvertFrom-Json -ErrorAction SilentlyContinue
        if ($o.type -ne 'assistant') { continue }
        foreach ($c in @($o.message.content)) { if ($c.type -eq 'text' -and $c.text) { $text = $c.text } }
    }
    $text
}
function ToolUses([string]$sid) {
    $t = Find-Transcript $sid
    if (-not $t) { return @() }
    $uses = @()
    foreach ($line in Get-Content $t) {
        $o = $line | ConvertFrom-Json -ErrorAction SilentlyContinue
        if ($o.type -ne 'assistant') { continue }
        foreach ($c in @($o.message.content)) { if ($c.type -eq 'tool_use') { $uses += "$($c.name): $($c.input.command)" } }
    }
    $uses
}

$names = [ordered]@{
    path  = "fm-selftest-$hex-path"
    allow = "fm-selftest-$hex-allow"
    bare  = "fm-selftest-$hex-bare"
    sid   = "fm-selftest-$hex-sid"
    ps    = "fm-selftest-$hex-ps"
    none  = "fm-selftest-$hex-none"
    ps2   = "fm-selftest-$hex-ps2"
}
$Only = @($Only | ForEach-Object { $_ -split "," })
foreach ($k in @($names.Keys)) { if ($Only -notcontains $k) { $names.Remove($k) } }
$uuid = [guid]::NewGuid().ToString()
$env:FOREMAN_HOME = $home_
try {
    Log "install: $(& $fm plugin install $plugin)"
    $settings = @{ env = @{ FOREMAN_HOME = $home_; FOREMAN_ROLE = 'probe'; FOREMAN_WORKSPACE = "selftest-$hex" }; worktree = @{ bgIsolation = 'none' } } | ConvertTo-Json -Compress
    $common = @('--strict-mcp-config', '--plugin-dir', $plugin, '--settings', $settings)
    $pPath = 'This is an automated Foreman probe. With your shell tool run exactly these two commands, each on its own: `where.exe fm` and then `fm --version`. Reply with both commands'' complete output verbatim, and the name of the tool you used. Do nothing else.'
    $pAllow = "This is an automated Foreman probe. With your Bash tool run exactly this command, unquoted: $fmPlugin --version . Reply with its output verbatim. Do nothing else."
    $pBare = 'This is an automated Foreman probe. With your Bash tool run exactly this command: fm --version . Reply with its output verbatim. Do nothing else.'
    Push-Location $cwd
    try {
        $pPs = 'This is an automated Foreman probe. With your PowerShell tool run exactly this command: fm --version . Reply with its output verbatim. Do nothing else.'
        $launch = @{
            path  = { claude --bg --name $names.path --permission-mode auto @common -- $pPath 2>&1 }
            allow = { claude --bg --name $names.allow --permission-mode default --allowedTools "Bash(${fmPlugin}:*)" @common -- $pAllow 2>&1 }
            bare  = { claude --bg --name $names.bare --permission-mode default --allowedTools 'Bash(fm:*)' @common -- $pBare 2>&1 }
            sid   = { claude --bg --name $names.sid --session-id $uuid --permission-mode auto @common -- 'Reply with the single word ready.' 2>&1 }
            ps    = { claude --bg --name $names.ps --permission-mode default --allowedTools 'PowerShell(fm:*)' @common -- $pPs 2>&1 }
            ps2   = { claude --bg --name $names.ps2 --permission-mode default --allowedTools 'PowerShell(fm *)' @common -- $pPs 2>&1 }
            none  = { claude --bg --name $names.none --permission-mode default @common -- $pBare 2>&1 }
        }
        foreach ($k in $names.Keys) { Log "launch $k : $((& $launch[$k]) -join ' | ')" }
    } finally { Pop-Location }

    $last = @{}
    $until = (Get-Date).AddSeconds($Seconds)
    while ((Get-Date) -lt $until) {
        $rows = Rows
        foreach ($k in $names.Keys) {
            $r = $rows | Where-Object { $_.name -eq $names[$k] } | Select-Object -First 1
            if (-not $r) { continue }
            $sig = "state=$($r.state) status=$($r.status) waitingFor=$($r.waitingFor) pid=$($r.pid) sid=$($r.sessionId) | last hook $(LastEvent $r.sessionId)"
            if ($last[$k] -ne $sig) { Log "row $k $sig"; $last[$k] = $sig }
        }
        Start-Sleep -Milliseconds 500
    }
    $rows = Rows
    foreach ($k in $names.Keys) {
        $r = $rows | Where-Object { $_.name -eq $names[$k] } | Select-Object -First 1
        if (-not $r) { Log "$k : no row"; continue }
        Log "$k tools: $((ToolUses $r.sessionId) -join ' || ')"
        Log "$k reply: $((LastText $r.sessionId) -replace "`r?`n", ' / ')"
    }
    $s = $rows | Where-Object { $_.name -eq $names.sid } | Select-Object -First 1
    if ($s) { Log "sid: asked $uuid, row sessionId $($s.sessionId), id $($s.id) -> $(if ($s.sessionId -eq $uuid) { 'SAME' } else { 'DIFFERENT' })" }
} catch {
    Log "ERROR: $_"
} finally {
    foreach ($r in @(Rows | Where-Object { $_.name -like "fm-selftest-$hex-*" })) {
        if ($r.pid) { Log "cleanup stop $($r.name): $(claude stop $r.id 2>&1)" }
    }
    Start-Sleep -Seconds 3
    foreach ($r in @(Rows | Where-Object { $_.name -like "fm-selftest-$hex-*" })) { Log "cleanup rm $($r.name): $(claude rm $r.id 2>&1)" }
    Log "left: $(@(Rows | Where-Object { $_.name -like "fm-selftest-$hex-*" }).Count)"
    Remove-Item -Recurse -Force $cwd -ErrorAction SilentlyContinue
    Log "log: $logPath"
}

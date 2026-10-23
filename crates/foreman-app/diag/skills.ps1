#Requires -Version 7
# m3/3: do sessions launched the way Foreman launches them see the foreman skills, and does the injected role
# name the skill? Sessions (cwd target\selftest-<hex>-skills, FOREMAN_HOME target\skills-diag-home, plugins/foreman
# with this build's fm.exe installed, --strict-mcp-config and no WorkState, bgIsolation none, auto,
# --allowedTools "Bash(fm:*)" "Bash(<abs fm>:*)"):
#   fm-selftest-<hex>-relay      the relay (roles/relay.md), so the workspace session's `ready` has somewhere to go
#   fm-selftest-<hex>-workspace  FOREMAN_ROLE=workspace, prompt = roles/workspace.md rendered (what Start launches)
#   fm-selftest-<hex>-repo       FOREMAN_ROLE=repo, prompt = roles/repo.md rendered + "First task:" (what fm want
#                                launches), the task a direct request: list the skills it can invoke
# Read back from each transcript: the skill_listing attachment (does it name foreman:workspace-controller and
# foreman:repo-controller?), the SessionStart injection (the FOREMAN-ROLE tag and the skill named in it), the tool
# calls of the first turn and the last reply; and the relay's outbox. Then every row is stopped and removed.
# -Drop: also launch fm-selftest-<hex>-target (the selftest's step r target, auto, no plugin) and, after the
# workspace session's `ready` reached the relay, drop one message for it with `fm send`; logs whether the relay
# delivered it (its answer in the outbox) or a notice came back (m3/3: the classifier refused the relay's send).
#   pwsh -File crates/foreman-app/diag/skills.ps1 [-TimeoutSec 240] [-Drop]
param([int]$TimeoutSec = 240, [switch]$Drop)
$ErrorActionPreference = 'Stop'

$root = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path
$hex = '{0:x6}' -f (Get-Random -Maximum 0xffffff)
$workspace = "selftest-$hex"
$home_ = Join-Path $root 'target\skills-diag-home'
$cwd = Join-Path $root "target\selftest-$hex-skills"
$repoDir = Join-Path $cwd 'Repo'
$plugin = Join-Path $root 'plugins\foreman'
$fm = Join-Path $root 'target\release\fm.exe'
function Fwd([string]$p) { $p -replace '\\', '/' }
$fmPlugin = Fwd (Join-Path $plugin 'bin\fm.exe')
$relayDir = Join-Path $home_ "relay\$workspace"
New-Item -ItemType Directory -Force $repoDir, (Join-Path $home_ 'roles'), (Join-Path $relayDir 'inbox'), (Join-Path $relayDir 'outbox') | Out-Null
$logPath = Join-Path $home_ "skills-$hex.log"

$t0 = Get-Date
function Log([string]$msg) {
    $line = '[{0:HH:mm:ss.fff} +{1,7:0.0}s] {2}' -f (Get-Date), ((Get-Date) - $t0).TotalSeconds, $msg
    Write-Host $line
    Add-Content -Path $logPath -Value $line
}
function Rows { @(claude agents --json --all | ConvertFrom-Json) }
function Find-Transcript([string]$sid) {
    Get-ChildItem -Path (Join-Path $HOME '.claude/projects') -Filter "$sid.jsonl" -File -Recurse -Depth 1 -ErrorAction SilentlyContinue | Select-Object -First 1 -ExpandProperty FullName
}
function Stopped([string]$sid) {
    $f = Join-Path $home_ "events\$sid.jsonl"
    (Test-Path $f) -and (@(Get-Content $f | Where-Object { $_ -like '*"event":"Stop"*' }).Count -gt 0)
}
function Role([string]$role, [string]$repo) {
    $template = (Get-Content (Join-Path $plugin "roles\$role.md") -Raw).Replace('{fm}', $fmPlugin).Replace('{home}', (Fwd $home_))
    Set-Content -Path (Join-Path $home_ "roles\$role.md") -Value $template -Encoding utf8NoBOM
    $template.Replace('{repo}', $repo).Replace('{workspace}', $workspace).Trim()
}
function Report([string]$k, [string]$sid) {
    $t = Find-Transcript $sid
    if (-not $t) { Log "$k : no transcript for $sid"; return }
    $listing = $null; $tag = $null; $text = ''; $uses = @()
    foreach ($line in Get-Content $t) {
        if (-not $listing -and $line -like '*"type":"skill_listing"*') { $listing = $line }
        if (-not $tag) { $m = [regex]::Match($line, 'FOREMAN-ROLE [a-z]+ v[0-9a-f]{8}'); if ($m.Success) { $tag = $m.Value; $tagLine = $line } }
        $o = $line | ConvertFrom-Json -ErrorAction SilentlyContinue
        if ($o.type -ne 'assistant') { continue }
        foreach ($c in @($o.message.content)) {
            if ($c.type -eq 'text' -and $c.text) { $text = $c.text }
            if ($c.type -eq 'tool_use') { $uses += "$($c.name): $(if ($c.input.skill) { $c.input.skill } elseif ($c.input.to) { "to=$($c.input.to) $($c.input.message -replace "`r?`n", ' / ')" } else { $c.input.command })" }
        }
    }
    $names = @('foreman:workspace-controller', 'foreman:repo-controller') | Where-Object { $listing -and $listing.Contains($_) }
    $skillInTag = if ($tagLine) { [regex]::Matches($tagLine, '/foreman:[a-z-]+') | ForEach-Object Value | Select-Object -Unique } else { @() }
    Log "$k transcript $t"
    Log "$k skill_listing names: $(if ($names) { $names -join ', ' } else { 'NONE' })"
    Log "$k injection: $tag, names $(($skillInTag) -join ', ')"
    Log "$k tools: $($uses -join ' || ')"
    Log "$k reply: $($text -replace "`r?`n", ' / ')"
}

$names = [ordered]@{ relay = "fm-$workspace-relay"; workspace = "fm-$workspace-workspace"; repo = "fm-$workspace-repo" }
$targetName = "fm-$workspace-target"
$env:FOREMAN_HOME = $home_
try {
    Log "install: $(& $fm plugin install $plugin)"
    $common = @('--strict-mcp-config', '--plugin-dir', $plugin, '--allowedTools', 'Bash(fm:*)', "Bash(${fmPlugin}:*)")
    function Settings([string]$role, [string]$repo) {
        $envs = @{ FOREMAN_HOME = $home_; FOREMAN_ROLE = $role; FOREMAN_WORKSPACE = $workspace }
        if ($repo) { $envs.FOREMAN_REPO = $repo }
        @{ env = $envs; worktree = @{ bgIsolation = 'none' } } | ConvertTo-Json -Compress
    }
    $relayPrompt = Role 'relay' ''
    $wsPrompt = Role 'workspace' ''
    $repoPrompt = (Role 'repo' 'Repo') + "`n`nFirst task:`nList every skill you can invoke whose name starts with foreman: or rlarch:, one per line, and reply with that list. Then stop."
    Push-Location $cwd
    try {
        Log "launch relay: $((claude --bg --name $names.relay --permission-mode auto @common --add-dir $relayDir --settings (Settings 'relay' '') -- $relayPrompt 2>&1) -join ' | ')"
        Start-Sleep -Seconds 2
        Log "launch workspace: $((claude --bg --name $names.workspace --permission-mode auto @common --settings (Settings 'workspace' '') -- $wsPrompt 2>&1) -join ' | ')"
    } finally { Pop-Location }
    if ($Drop) {
        Push-Location $cwd
        try {
            $tp = 'This is an automated Foreman selftest. When another session sends you a message, reply to its sender with SendMessage repeating the FOREMAN-RELAY nonce it contains, then wait for more. Do nothing else.'
            Log "launch target: $((claude --bg --name $targetName --permission-mode auto --strict-mcp-config --settings '{"worktree":{"bgIsolation":"none"}}' -- $tp 2>&1 | Select-Object -First 1))"
        } finally { Pop-Location }
    }
    Push-Location $repoDir
    try {
        Log "launch repo: $((claude --bg --name $names.repo --permission-mode auto @common --settings (Settings 'repo' 'Repo') -- $repoPrompt 2>&1) -join ' | ')"
    } finally { Pop-Location }

    $done = @{}
    $last = @{}
    $deadline = (Get-Date).AddSeconds($TimeoutSec)
    while ((Get-Date) -lt $deadline -and $done.Count -lt 2) {
        $rows = Rows
        foreach ($k in $names.Keys) {
            $r = $rows | Where-Object { $_.name -eq $names[$k] } | Select-Object -First 1
            if (-not $r) { continue }
            $sig = "state=$($r.state) status=$($r.status) waitingFor=$($r.waitingFor) pid=$($r.pid)"
            if ($last[$k] -ne $sig) { Log "row $k $sig"; $last[$k] = $sig }
            if ($k -ne 'relay' -and -not $done[$k] -and (Stopped $r.sessionId)) { $done[$k] = $true; Log "$k first turn ended" }
        }
        Start-Sleep -Milliseconds 500
    }
    # the workspace session's `ready` may still be on its way through the relay
    Start-Sleep -Seconds 15
    $rows = Rows
    foreach ($k in @('workspace', 'repo')) {
        $r = $rows | Where-Object { $_.name -eq $names[$k] } | Select-Object -First 1
        if ($r) { Log "$k final row state=$($r.state) status=$($r.status) waitingFor=$($r.waitingFor)"; Report $k $r.sessionId } else { Log "$k : no row" }
    }
    if ($Drop) {
        $nonce = "FOREMAN-RELAY-$('{0:x6}' -f (Get-Random -Maximum 0xffffff))"
        Log "drop: $(& $fm send $workspace $targetName "$nonce ping from the diag`nsecond line" 2>&1)"
        $until = (Get-Date).AddSeconds(120); $got = $null
        while (-not $got -and (Get-Date) -lt $until) {
            Start-Sleep -Milliseconds 500
            $got = Get-ChildItem (Join-Path $relayDir 'outbox') -Filter 'msg-*.json' | Where-Object { (Get-Content $_.FullName -Raw) -match "$nonce|relay-notice" } | Select-Object -First 1
        }
        Log "drop result: $(if ($got) { "$($got.Name): $((Get-Content $got.FullName -Raw) -replace "`r?`n", ' ')" } else { 'nothing within 120 s' })"
    }
    foreach ($f in @(Get-ChildItem (Join-Path $relayDir 'outbox') -Filter 'msg-*.json' -ErrorAction SilentlyContinue)) {
        Log "outbox $($f.Name): $((Get-Content $f.FullName -Raw) -replace "`r?`n", ' ')"
    }
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

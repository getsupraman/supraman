# m2/4: `claude --bg --resume <sessionId>` (no other flag, no prompt) after the session's process was
# killed (-How kill: taskkill /PID of the row's pid) or stopped (-How stop: claude stop). What are the row's
# id / sessionId / pid afterwards, and which transcript files exist in ~/.claude/projects/<cwd slug>?
# One fm-selftest-resume-<hex> session, cwd target\diag-resume; stopped and removed at the end.
#   pwsh -File crates/foreman-app/diag/resume.ps1 [-How kill|stop] [-Delay <s after the pid dropped>]
param([ValidateSet('kill', 'stop')][string]$How = 'kill', [int]$Delay = 0)
$ErrorActionPreference = 'Stop'
$root = Resolve-Path (Join-Path $PSScriptRoot '..\..\..')
$cwd = Join-Path $root 'target\diag-resume'
New-Item -ItemType Directory -Force $cwd | Out-Null
$name = 'fm-selftest-resume-{0:x6}' -f (Get-Random -Maximum 0xffffff)
$settings = '{"worktree":{"bgIsolation":"none"}}'

function Row { (claude agents --json --all | ConvertFrom-Json) | Where-Object { $_.name -eq $name } | Select-Object -First 1 }
function Show($label, $row) { "$label id=$($row.id) sid=$($row.sessionId) state=$($row.state) status=$($row.status) pid=$($row.pid) cwd=$($row.cwd)" }
function Transcripts {
    $slug = ($cwd -replace '[:\\/.]', '-')
    $dir = Join-Path $HOME ".claude\projects\$slug"
    if (Test-Path $dir) { Get-ChildItem $dir -Filter *.jsonl | ForEach-Object { "  transcript $($_.Name) $($_.Length) bytes" } } else { "  no $dir" }
}

Push-Location $cwd
try { $out = claude --bg --name $name --permission-mode auto --strict-mcp-config --settings $settings -- 'Reply with the word ready and then wait for input' } finally { Pop-Location }
"launched: $($out -split "`n" | Select-Object -First 1)"
try {
    $deadline = (Get-Date).AddSeconds(60)
    do { Start-Sleep -Milliseconds 500; $row = Row } until (($row.pid -and $row.state -eq 'done') -or (Get-Date) -gt $deadline)
    Show 'idle:' $row
    $old = $row.sessionId
    if ($How -eq 'kill') { taskkill /PID $row.pid /F | Out-Null } else { claude stop $row.id | Out-Null }
    $deadline = (Get-Date).AddSeconds(20)
    do { Start-Sleep -Milliseconds 500; $row = Row } until (-not $row.pid -or (Get-Date) -gt $deadline)
    Show "after ${How}:" $row
    Transcripts
    Start-Sleep -Seconds $Delay
    Show "after ${Delay} s:" (Row)
    Push-Location $cwd
    try { $out = claude --bg --resume $old } finally { Pop-Location }
    "resume: $($out -join ' | ')"
    $deadline = (Get-Date).AddSeconds(30)
    do { Start-Sleep -Milliseconds 500; $row = Row } until ($row.pid -or (Get-Date) -gt $deadline)
    Show 'resumed:' $row
    Start-Sleep -Seconds 5
    Show 'resumed +5 s:' (Row)
    Transcripts
    $new = (Row).sessionId
    $slug = ($cwd -replace '[:\\/.]', '-')
    $file = Join-Path $HOME ".claude\projects\$slug\$new.jsonl"
    if (Test-Path $file) { "new transcript mentions old sessionId: $([bool](Select-String -Path $file -SimpleMatch $old -Quiet))" }
} finally {
    $all = (claude agents --json --all | ConvertFrom-Json) | Where-Object { $_.name -eq $name }
    foreach ($r in $all) {
        if ($r.pid) { claude stop $r.id | Out-Null; Start-Sleep -Seconds 2 }
        "cleanup: $(claude rm $r.id)"
    }
    Remove-Item -Recurse -Force $cwd -ErrorAction SilentlyContinue
}

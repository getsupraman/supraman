# m2/1 probe (d) by hand: which `claude --bg --resume` form keeps the sessionId?
# Launches one aw-m2-probe-* session from target/, stops it, resumes it with -Variant, prints the rows, removes them.
param([ValidateSet('bare', 'prompt', 'name', 'name-prompt')] [string] $Variant = 'bare')
$ErrorActionPreference = 'Stop'
$hex = '{0:x6}' -f (Get-Random -Maximum 0xffffff)
$name = "aw-m2-probe-$hex-r"
$cwd = Join-Path $PSScriptRoot "..\..\..\target\aw-m2-probe-$hex"
New-Item -ItemType Directory -Force $cwd | Out-Null

function Rows { (claude agents --json --all | ConvertFrom-Json) | Where-Object name -eq $name }
function Show($label) { Rows | ForEach-Object { "$label id=$($_.id) sessionId=$($_.sessionId) name=$($_.name) pid=$($_.pid) state=$($_.state)" } }

Push-Location $cwd
try {
    claude --bg --name $name --permission-mode auto -- 'Reply with the word ready and stop.'
    $deadline = (Get-Date).AddSeconds(60)
    while ((Get-Date) -lt $deadline -and -not (Rows | Where-Object { $_.pid -and $_.state -eq 'done' })) { Start-Sleep -Milliseconds 500 }
    Show 'before'
    $sid = (Rows | Select-Object -First 1).sessionId
    claude stop (Rows | Select-Object -First 1).id
    Start-Sleep -Seconds 2
    Show 'stopped'
    $resume = switch ($Variant) {
        'bare' { @('--bg', '--resume', $sid) }
        'prompt' { @('--bg', '--resume', $sid, '--', 'Reply with the word again and stop.') }
        'name' { @('--bg', '--name', $name, '--resume', $sid) }
        'name-prompt' { @('--bg', '--name', $name, '--resume', $sid, '--', 'Reply with the word again and stop.') }
    }
    "resume: claude $($resume -join ' ')"
    claude @resume
    "exit $LASTEXITCODE"
    Start-Sleep -Seconds 6
    Show 'after'
    claude agents --json --all | ConvertFrom-Json | Where-Object sessionId -eq $sid | ForEach-Object { "by-sessionId id=$($_.id) name=$($_.name) pid=$($_.pid) state=$($_.state)" }
} finally {
    Pop-Location
    Rows | ForEach-Object { claude stop $_.id | Out-Null; claude rm $_.id }
    claude agents --json --all | ConvertFrom-Json | Where-Object { $_.sessionId -eq $sid -and $_.name -ne $name } | ForEach-Object { claude stop $_.id | Out-Null; claude rm $_.id }
}

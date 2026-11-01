#Requires -Version 7
# sched2/11 probe: does an interactive claude in a Supraman-like terminal emit OSC 8 for a markdown link,
# with and without FORCE_HYPERLINK=1? Runs claude.exe on a ConPTY (ConPty.cs) with the tab env of
# supraman-term builder.rs, sends one prompt, keeps the raw output bytes and reports whether ESC ]8; appears.
# Plan mode, no MCP servers, hooks off, a throwaway cwd under the worktree's target; ends claude by its pid.
# Run from the repo root: pwsh -File probes/sched2.11-osc8/run.ps1 [-Claude <claude.exe>] [-Model haiku]
param(
    [string]$Claude = (Join-Path $env:USERPROFILE '.local\bin\claude.exe'),
    [string]$Model = 'haiku'
)

$ErrorActionPreference = 'Stop'
$root = Resolve-Path (Join-Path $PSScriptRoot '../..')
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$scratch = Join-Path $PSScriptRoot ".scratch/$stamp"
$cwd = Join-Path $root "target/osc8-probe/$stamp"
$home_ = Join-Path $scratch 'supraman-home'
New-Item -ItemType Directory -Force $scratch, $cwd, $home_ | Out-Null
$settings = Join-Path $scratch 'settings.json'
'{"disableAllHooks":true}' | Set-Content -NoNewline $settings

Add-Type -Path (Join-Path $PSScriptRoot 'ConPty.cs')

$url = 'https://example.com/osc8-probe'
$prompt = "Reply with exactly this markdown and nothing else: [Supraman]($url)"

function Set-TabEnv([bool]$force) {
    foreach ($v in 'WT_SESSION', 'WT_PROFILE_ID', 'TERM_PROGRAM', 'TERM_PROGRAM_VERSION', 'FORCE_HYPERLINK',
                   'VTE_VERSION', 'CLAUDECODE', 'CLAUDE_CODE_ENTRYPOINT', 'ZED_TERM', 'CI', 'NETLIFY') {
        Remove-Item "Env:$v" -ErrorAction SilentlyContinue
    }
    $env:TERM_PROGRAM = 'supraman'
    $env:TERM_PROGRAM_VERSION = '0.0.0-probe'
    $env:TERM = 'xterm-256color'
    $env:COLORTERM = 'truecolor'
    $env:CLAUDE_CODE_FORCE_SYNC_OUTPUT = '1'
    $env:CLAUDE_CODE_FORCE_STRIKETHROUGH = '1'
    $env:SUPRAMAN_HOME = $home_
    $env:FOREMAN_HOME = $home_
    if ($force) { $env:FORCE_HYPERLINK = '1' }
}

function Invoke-Run([string]$label, [bool]$force) {
    Set-TabEnv $force
    $cmd = "`"$Claude`" --permission-mode plan --strict-mcp-config --settings `"$settings`" --model $Model"
    $p = [ConPtyProbe]::new($cmd, $cwd, 120, 40)
    $out = Join-Path $scratch "$label.bin"
    $verdict = 'timeout'
    try {
        $deadline = (Get-Date).AddSeconds(25)
        while ((Get-Date) -lt $deadline) {
            Start-Sleep -Milliseconds 500
            $t = $p.Text()
            if ($t -match '(?i)trust the files|Do you trust') { $verdict = 'trust-dialog (not answered)'; break }
            if ($t -match 'for shortcuts|bypass|plan mode') { break }
        }
        if ($verdict -eq 'timeout') {
            Start-Sleep -Seconds 2
            $p.Write($prompt)
            Start-Sleep -Milliseconds 800
            $p.Write("`r")
            $sent = $p.Bytes().Length
            $deadline = (Get-Date).AddSeconds(90)
            while ((Get-Date) -lt $deadline) {
                Start-Sleep -Milliseconds 500
                $b = $p.Bytes()
                $after = [Text.Encoding]::UTF8.GetString($b, $sent, $b.Length - $sent)
                if ($after -match "`e\]8;[^;]*;$([regex]::Escape($url))" -or $after -match 'Supraman \(https://example\.com/osc8-probe\)') {
                    Start-Sleep -Seconds 2; $verdict = 'answered'; break
                }
            }
        }
        $p.Write("`u{3}"); Start-Sleep -Milliseconds 400; $p.Write("`u{3}"); Start-Sleep -Seconds 2
    }
    finally {
        $proc = Get-Process -Id $p.Pid -ErrorAction SilentlyContinue
        if ($proc -and $proc.ProcessName -eq 'claude') { Stop-Process -Id $p.Pid -Force }
        $left = if (Get-Process -Id $p.Pid -ErrorAction SilentlyContinue) { 'still running' } else { 'ended' }
        [IO.File]::WriteAllBytes($out, $p.Bytes())
        $p.Dispose()
    }
    $text = [IO.File]::ReadAllText($out)
    $osc8 = [regex]::Matches($text, "`e\]8;[^;]*;[^`a`e]*")
    "{0,-22} {1}; pid {5} {6}; {2} bytes; ESC]8; x{3}{4}" -f $label, $verdict, (Get-Item $out).Length, $osc8.Count,
        $(if ($osc8.Count) { ': ' + (($osc8 | ForEach-Object { $_.Value.Substring(1) } | Select-Object -Unique) -join ' | ') }), $p.Pid, $left
    $plain = [regex]::Match($text, 'Supraman \(https://example\.com/osc8-probe\)')
    if ($plain.Success) { "{0,-22} plain fallback seen: {1}" -f '', $plain.Value }
}

"claude:  $Claude ($((& $Claude --version) -join ' '))"
"cwd:     $cwd"
Invoke-Run 'without-FORCE' $false
Invoke-Run 'with-FORCE_HYPERLINK=1' $true
"bytes:   $scratch"

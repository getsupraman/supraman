# Times `fm hook` (release build) on a sample SessionStart payload, the slowest path (log append + role
# read + stdout). Scratch FOREMAN_HOME under target/. Usage: pwsh -File crates/fm/probe/hook-timing.ps1 [-N 50]
param([int]$N = 50)
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path "$PSScriptRoot/../../..").Path
$fm = Join-Path $root 'target/release/fm.exe'
if (-not (Test-Path $fm)) { throw "build first: cargo build --release -p fm" }
$home_ = Join-Path $root "target/aw-m2-hook-timing"
New-Item -ItemType Directory -Force (Join-Path $home_ 'roles') | Out-Null
Set-Content -NoNewline -Path (Join-Path $home_ 'roles/timing.md') -Value ("You are the timing role.`n" * 40)
$payload = '{"cwd":"C:\\w","hook_event_name":"SessionStart","model":"m","session_id":"0a1b2c3d-0000-4000-8000-0000000000ff","session_title":"aw-timing","source":"compact","transcript_path":"t"}'
$psi = [System.Diagnostics.ProcessStartInfo]::new($fm)
$psi.ArgumentList.Add('hook'); $psi.ArgumentList.Add('SessionStart')
$psi.RedirectStandardInput = $true; $psi.RedirectStandardOutput = $true; $psi.UseShellExecute = $false
$psi.Environment['FOREMAN_HOME'] = $home_; $psi.Environment['FOREMAN_ROLE'] = 'timing'
$ms = @()
for ($i = 0; $i -lt $N; $i++) {
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $p = [System.Diagnostics.Process]::Start($psi)
    $p.StandardInput.Write($payload); $p.StandardInput.Close()
    $out = $p.StandardOutput.ReadToEnd(); $p.WaitForExit()
    $sw.Stop()
    if ($p.ExitCode -ne 0 -or -not $out.StartsWith('FOREMAN-ROLE timing v')) { throw "bad run: exit $($p.ExitCode) out '$out'" }
    $ms += $sw.Elapsed.TotalMilliseconds
}
$s = $ms | Sort-Object
"fm hook SessionStart x$N (spawn to exit): min {0:N1} ms, median {1:N1} ms, p95 {2:N1} ms, max {3:N1} ms" -f $s[0], $s[[int]($N / 2)], $s[[int]($N * 0.95) - 1], $s[-1]

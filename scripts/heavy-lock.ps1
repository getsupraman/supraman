# Runs a heavy job (a cargo build or test) under the machine-wide lock every selftest takes
# (supraman_core::fs::heavy_job_lock): a build waits for a running selftest and the other way round.
#   pwsh -File scripts/heavy-lock.ps1 cargo test --release -p supraman-core
# No param block: every argument, `-p` included, goes to the program untouched.
if ($args.Count -lt 1) { [Console]::Error.WriteLine('usage: heavy-lock.ps1 <program> [arguments...]'); exit 2 }
$program = $args[0]
# wasmtime-c-api-impl's build script (Zed's `language` needs tree-sitter's `wasm`) runs a bare `cmake`; Visual Studio bundles one.
if (-not (Get-Command cmake -ErrorAction SilentlyContinue)) {
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    if (Test-Path $vswhere) {
        foreach ($vs in @(& $vswhere -all -products * -property installationPath)) {
            $bin = Join-Path $vs 'Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin'
            if (Test-Path (Join-Path $bin 'cmake.exe')) { $env:PATH = "$bin;$env:PATH"; break }
        }
    }
}
$rest = @($args | Select-Object -Skip 1)
$path = Join-Path ([IO.Path]::GetTempPath()) 'supraman-heavy.lock'
$file = [IO.File]::Open($path, 'OpenOrCreate', 'ReadWrite', 'ReadWrite')
$said = $false
while ($true) {
    try { $file.Lock(0, [long]::MaxValue); break }
    catch [IO.IOException] {
        if (-not $said) { [Console]::Error.WriteLine("$program waits for another heavy job on this machine to finish (it holds $path)"); $said = $true }
        Start-Sleep -Milliseconds 500
    }
}
# Worktrees share one target dir, where a member's artifacts are named by its path below the root: a switch cleans them.
function Clear-SwitchedMembers {
    if ([IO.Path]::GetFileNameWithoutExtension($program) -ne 'cargo') { return }
    $sub = $rest | Where-Object { $_ -notlike '-*' -and $_ -notlike '+*' } | Select-Object -First 1
    if ($sub -notin 'build', 'b', 'test', 't', 'run', 'r', 'check', 'c', 'clippy', 'bench', 'doc', 'd', 'rustc', 'rustdoc') { return }
    $where = @()
    $targetDir = $null
    for ($i = 0; $i -lt $rest.Count; $i++) {
        $a = [string]$rest[$i]
        if ($a -eq '--manifest-path' -and $i + 1 -lt $rest.Count) { $where = @('--manifest-path', $rest[$i + 1]) }
        elseif ($a -like '--manifest-path=*') { $where = @('--manifest-path', $a.Substring(16)) }
        elseif ($a -eq '--target-dir' -and $i + 1 -lt $rest.Count) { $targetDir = [string]$rest[$i + 1] }
        elseif ($a -like '--target-dir=*') { $targetDir = $a.Substring(13) }
    }
    $json = & $program (@('metadata', '--no-deps', '--format-version', '1', '--offline') + $where) 2>$null
    if ($LASTEXITCODE -ne 0 -or -not $json) { return }
    $meta = $json | ConvertFrom-Json
    $root = [IO.Path]::GetFullPath($meta.workspace_root).TrimEnd('\')
    if (-not $targetDir) { $targetDir = $meta.target_directory }
    $targetDir = [IO.Path]::GetFullPath($targetDir)
    if (-not (Test-Path $targetDir)) { return }
    $marker = Join-Path $targetDir '.last-workspace'
    $last = if (Test-Path $marker) { (Get-Content -Raw $marker).Trim() } else { '' }
    if ($last -ieq $root) { return }
    $members = @($meta.packages | Where-Object { $meta.workspace_members -contains $_.id } | ForEach-Object { '-p'; $_.name })
    $profiles = @('debug', 'release') | Where-Object { Test-Path (Join-Path $targetDir $_) }
    $clock = [Diagnostics.Stopwatch]::StartNew()
    foreach ($dir in $profiles) {
        $clean = @('clean', '--quiet', '--offline', '--target-dir', $targetDir) + $where + $members
        if ($dir -eq 'release') { $clean += '--release' }
        $out = & $program $clean 2>&1
        if ($LASTEXITCODE -ne 0) { [Console]::Error.WriteLine("heavy-lock: cargo clean failed, the marker stays: $out"); return }
    }
    [IO.File]::WriteAllText($marker, $root)
    $was = if ($last) { $last } else { 'unknown' }
    [Console]::Error.WriteLine("heavy-lock: $targetDir last built $was, now $root; cleaned $($members.Count / 2) workspace members ($($profiles -join ', ')) in $([int]$clock.Elapsed.TotalSeconds) s")
}
try {
    try { Clear-SwitchedMembers } catch { [Console]::Error.WriteLine("heavy-lock: no switch check: $_") }
    & $program @rest; $code = $LASTEXITCODE
}
finally { $file.Unlock(0, [long]::MaxValue); $file.Dispose() }
exit $code

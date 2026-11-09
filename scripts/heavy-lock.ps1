# Runs a heavy job (a cargo build or test) under the machine-wide lock every selftest takes
# (supraman_core::fs::heavy_job_lock): a build waits for a running selftest and the other way round.
#   pwsh -File scripts/heavy-lock.ps1 cargo test --release -p supraman-core
# No param block: every argument, `-p` included, goes to the program untouched.
# Dot-sourced (`. scripts/heavy-lock.ps1`), it only defines its functions, for a check on a saved log.

# What a cargo command line builds: its subcommand, profile, profile folder and the flags metadata and clean need.
function Get-CargoRun([string]$program, [object[]]$rest) {
    if ([IO.Path]::GetFileNameWithoutExtension($program) -ne 'cargo') { return $null }
    $sub = $rest | Where-Object { $_ -notlike '-*' -and $_ -notlike '+*' } | Select-Object -First 1
    if ($sub -notin 'build', 'b', 'test', 't', 'run', 'r', 'check', 'c', 'clippy', 'bench', 'doc', 'd', 'rustc', 'rustdoc') { return $null }
    $run = @{ sub = $sub; where = @(); targetDir = $null; triple = $null; prof = $(if ($sub -eq 'bench') { 'bench' } else { 'dev' }) }
    for ($i = 0; $i -lt $rest.Count; $i++) {
        $a = [string]$rest[$i]
        if ($a -eq '--manifest-path' -and $i + 1 -lt $rest.Count) { $run.where += @('--manifest-path', $rest[$i + 1]) }
        elseif ($a -like '--manifest-path=*') { $run.where += @('--manifest-path', $a.Substring(16)) }
        elseif ($a -eq '--target-dir' -and $i + 1 -lt $rest.Count) { $run.targetDir = [string]$rest[$i + 1] }
        elseif ($a -like '--target-dir=*') { $run.targetDir = $a.Substring(13) }
        elseif ($a -eq '--target' -and $i + 1 -lt $rest.Count) { $run.triple = [string]$rest[$i + 1] }
        elseif ($a -like '--target=*') { $run.triple = $a.Substring(9) }
        elseif ($a -ceq '--release' -or $a -ceq '-r') { $run.prof = 'release' }
        elseif ($a -eq '--profile' -and $i + 1 -lt $rest.Count) { $run.prof = [string]$rest[$i + 1] }
        elseif ($a -like '--profile=*') { $run.prof = $a.Substring(10) }
        elseif ($a -eq '--config' -and $i + 1 -lt $rest.Count) { $run.where += @('--config', $rest[$i + 1]) }
        elseif ($a -like '--config=*') { $run.where += @('--config', $a.Substring(9)) }
    }
    $run.dir = switch ($run.prof) { 'dev' { 'debug' } 'test' { 'debug' } 'bench' { 'release' } default { $run.prof } }
    $json = & $program (@('metadata', '--no-deps', '--format-version', '1', '--offline') + $run.where) 2>$null
    if ($LASTEXITCODE -ne 0 -or -not $json) { return $null }
    $meta = $json | ConvertFrom-Json
    $run.root = [IO.Path]::GetFullPath($meta.workspace_root).TrimEnd('\')
    if (-not $run.targetDir) { $run.targetDir = $meta.target_directory }
    $run.targetDir = [IO.Path]::GetFullPath($run.targetDir)
    $run.members = @($meta.packages | Where-Object { $meta.workspace_members -contains $_.id })
    $run.crates = @{}
    $run.linked = @{}
    foreach ($m in $run.members) {
        foreach ($t in $m.targets) {
            $crate = $t.name -replace '-', '_'
            $run.crates[$crate] = $m.name
            if (@($t.kind | Where-Object { $_ -in 'lib', 'rlib', 'dylib', 'proc-macro', 'bin' }).Count) { $run.linked[$crate] = $m.name }
        }
    }
    $run
}

# Worktrees share one target dir, where a member's artifacts are named by its path below the root: a switch cleans
# them. debug and release share the root's marker (older copies of this script read it); any other profile, such as
# unit, keeps its own in its folder, so a release build elsewhere leaves a unit build's members alone.
function Clear-SwitchedMembers($run) {
    $targetDir = $run.targetDir
    $prof = $run.prof
    if (-not (Test-Path $targetDir)) { return }
    if ($prof -in 'dev', 'test', 'release', 'bench') {
        $marker = Join-Path $targetDir '.last-workspace'
        $profiles = @(@{ dir = 'debug'; name = 'dev' }, @{ dir = 'release'; name = 'release' })
    } else {
        $built = Join-Path $targetDir $prof
        if (-not (Test-Path $built)) { New-Item -ItemType Directory $built | Out-Null; [IO.File]::WriteAllText((Join-Path $built '.last-workspace'), $run.root); return }
        $marker = Join-Path $built '.last-workspace'
        $profiles = @(@{ dir = $prof; name = $prof })
    }
    $last = if (Test-Path $marker) { (Get-Content -Raw $marker).Trim() } else { '' }
    if ($last -ieq $run.root) { return }
    $members = @($run.members | ForEach-Object { '-p'; $_.name })
    $cleaned = @($profiles | Where-Object { Test-Path (Join-Path $targetDir $_.dir) })
    $clock = [Diagnostics.Stopwatch]::StartNew()
    foreach ($p in $cleaned) {
        $out = & $program (@('clean', '--quiet', '--offline', '--target-dir', $targetDir, '--profile', $p.name) + $run.where + $members) 2>&1
        if ($LASTEXITCODE -ne 0) { [Console]::Error.WriteLine("heavy-lock: cargo clean failed, the marker stays: $out"); return }
    }
    [IO.File]::WriteAllText($marker, $run.root)
    $was = if ($last) { $last } else { 'unknown' }
    [Console]::Error.WriteLine("heavy-lock: $targetDir last built $was, now $($run.root); cleaned $($members.Count / 2) workspace members ($(($cleaned | ForEach-Object { $_.dir }) -join ', ')) in $([int]$clock.Elapsed.TotalSeconds) s")
}

function Get-IncrementalDir($run) {
    $base = if ($run.triple) { Join-Path $run.targetDir $run.triple } else { $run.targetDir }
    Join-Path (Join-Path $base $run.dir) 'incremental'
}

# A member's newest session left `-working`: rustc could not rename it to its final name (Defender holding a file,
# docs/research/refactor4-audit.md L-1), and the crate's next build would start from an older session (L-2).
function Get-UnfinalizedCrates([string]$incremental, [hashtable]$crates) {
    if (-not (Test-Path $incremental)) { return }
    foreach ($d in Get-ChildItem -LiteralPath $incremental -Directory) {
        if ($d.Name -notmatch '^(.+)-[0-9a-z]+$' -or -not $crates.ContainsKey($Matches[1])) { continue }
        $newest = Get-ChildItem -LiteralPath $d.FullName -Directory -Filter 's-*' |
            Sort-Object { $_.Name.Split('-')[1].Length }, { $_.Name.Split('-')[1] } | Select-Object -Last 1
        if ($newest -and $newest.Name.EndsWith('-working')) { [pscustomobject]@{ dir = $d; session = $newest.Name } }
    }
}

# The rename fails while any process holds a file below the folder, so a held crate is left alone.
function Remove-UnfinalizedSessions($run) {
    $incremental = Get-IncrementalDir $run
    if (-not (Test-Path $incremental)) { return }
    foreach ($old in Get-ChildItem -LiteralPath $incremental -Directory -Filter '*.heavy-lock-stale') {
        Remove-Item -LiteralPath $old.FullName -Recurse -Force -ErrorAction SilentlyContinue
    }
    foreach ($u in @(Get-UnfinalizedCrates $incremental $run.crates)) {
        $aside = "$($u.dir.FullName).heavy-lock-stale"
        try { [IO.Directory]::Move($u.dir.FullName, $aside) }
        catch { [Console]::Error.WriteLine("heavy-lock: kept $($u.dir.Name): its newest session $($u.session) never finalized, and a process holds a file in it"); continue }
        Remove-Item -LiteralPath $aside -Recurse -Force -ErrorAction SilentlyContinue
        [Console]::Error.WriteLine("heavy-lock: removed $incremental\$($u.dir.Name): its newest session $($u.session) never finalized; the crate compiles in full next time")
    }
}

# The members whose library or binary crate names a linker's LNK2019 or LNK2001 line holds: a mangled path segment
# is <len><name>, a type inside generic arguments is spelt `name..path`.
function Find-LinkMembers([string[]]$lines, [hashtable]$crates) {
    $found = @{}
    foreach ($line in $lines) {
        if ($line -notmatch 'LNK20(19|01)') { continue }
        foreach ($crate in $crates.Keys) {
            $name = [regex]::Escape($crate)
            if ($line -match "(?<![0-9])$($crate.Length)_?$name(?![a-z_])|(?<![0-9A-Za-z_])$name(\.\.|::)") { $found[$crates[$crate]] = $true }
        }
    }
    @($found.Keys | Sort-Object)
}

function Format-Argument([string]$a) {
    if ($a -and $a -notmatch '[\s"]') { return $a }
    '"' + ($a -replace '(\\*)"', '$1$1\"' -replace '(\\+)$', '$1$1') + '"'
}

# cargo's stdout stays the caller's; its stderr goes on to the caller line by line, and the LNK lines are kept.
function Invoke-Watched([string]$program, [object[]]$rest) {
    $start = New-Object Diagnostics.ProcessStartInfo
    $start.FileName = (Get-Command $program -CommandType Application | Select-Object -First 1).Source
    $start.Arguments = ($rest | ForEach-Object { Format-Argument ([string]$_) }) -join ' '
    $start.UseShellExecute = $false
    $start.RedirectStandardError = $true
    $p = [Diagnostics.Process]::Start($start)
    $kept = New-Object Collections.Generic.List[string]
    while ($null -ne ($line = $p.StandardError.ReadLine())) {
        [Console]::Error.WriteLine($line)
        if ($line -match 'LNK(2019|2001|1120)') { $kept.Add($line) }
    }
    $p.WaitForExit()
    @{ code = $p.ExitCode; lnk = $kept.ToArray() }
}

function Invoke-Cargo([string]$program, [object[]]$rest, $run) {
    $result = Invoke-Watched $program $rest
    try { Remove-UnfinalizedSessions $run } catch { [Console]::Error.WriteLine("heavy-lock: no incremental check: $_") }
    if ($result.code -eq 0) { return 0 }
    $broken = @(Find-LinkMembers $result.lnk $run.linked)
    if ($broken.Count -eq 0) { return $result.code }
    [Console]::Error.WriteLine("heavy-lock: LNK2019 names $($broken -join ', '); cargo clean --profile $($run.prof) of those, then the same command once more")
    $out = & $program (@('clean', '--quiet', '--offline', '--target-dir', $run.targetDir, '--profile', $run.prof) + $run.where + @($broken | ForEach-Object { '-p'; $_ })) 2>&1
    if ($LASTEXITCODE -ne 0) { [Console]::Error.WriteLine("heavy-lock: cargo clean failed, no retry: $out"); return $result.code }
    $again = Invoke-Watched $program $rest
    try { Remove-UnfinalizedSessions $run } catch { [Console]::Error.WriteLine("heavy-lock: no incremental check: $_") }
    $again.code
}

if ($MyInvocation.InvocationName -eq '.') { return }

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
# Every worktree builds into the main checkout's target (docs/DEVELOPING.md); one under a *.worktrees path fails sm's dry-run test.
if (-not $env:CARGO_TARGET_DIR) {
    $common = git -C $PSScriptRoot rev-parse --path-format=absolute --git-common-dir 2>$null
    if ($LASTEXITCODE -eq 0 -and $common) { $env:CARGO_TARGET_DIR = Join-Path (Split-Path -Parent ([IO.Path]::GetFullPath($common))) 'target' }
}
# cargo colours its stderr only for a console; the pipe Invoke-Watched reads would turn that off.
if (-not [Console]::IsErrorRedirected -and -not $env:CARGO_TERM_COLOR) { $env:CARGO_TERM_COLOR = 'always' }
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
try {
    $run = $null
    try { $run = Get-CargoRun $program $rest } catch { [Console]::Error.WriteLine("heavy-lock: no cargo metadata: $_") }
    if ($run) {
        try { Clear-SwitchedMembers $run } catch { [Console]::Error.WriteLine("heavy-lock: no switch check: $_") }
        $code = Invoke-Cargo $program $rest $run
    } else {
        & $program @rest; $code = $LASTEXITCODE
    }
}
finally { $file.Unlock(0, [long]::MaxValue); $file.Dispose() }
exit $code

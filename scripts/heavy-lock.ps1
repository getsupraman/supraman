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
try { & $program @rest; $code = $LASTEXITCODE }
finally { $file.Unlock(0, [long]::MaxValue); $file.Dispose() }
exit $code

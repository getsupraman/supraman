# Runs a heavy job (a cargo build or test) under the machine-wide lock every selftest takes
# (supraman_core::fs::heavy_job_lock): a build waits for a running selftest and the other way round.
#   pwsh -File scripts/heavy-lock.ps1 cargo test --release --workspace
param([Parameter(Mandatory, Position = 0)][string]$Program, [Parameter(ValueFromRemainingArguments)][string[]]$Arguments)
$path = Join-Path ([IO.Path]::GetTempPath()) 'supraman-heavy.lock'
$file = [IO.File]::Open($path, 'OpenOrCreate', 'ReadWrite', 'ReadWrite')
$said = $false
while ($true) {
    try { $file.Lock(0, [long]::MaxValue); break }
    catch [IO.IOException] {
        if (-not $said) { [Console]::Error.WriteLine("$Program waits for another heavy job on this machine to finish (it holds $path)"); $said = $true }
        Start-Sleep -Milliseconds 500
    }
}
try { & $Program @Arguments; $code = $LASTEXITCODE }
finally { $file.Unlock(0, [long]::MaxValue); $file.Dispose() }
exit $code

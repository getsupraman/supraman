#Requires -Version 7
# M0.3 hook: append one JSON line {ts, event, payload[, nonce]} to -Out, payload = stdin verbatim.
# On SessionStart, also print a fresh SUPRAMAN-NONCE-<hex> to stdout (plain stdout is added to context).
param(
    [Parameter(Mandatory)][string]$Event,
    [Parameter(Mandatory)][string]$Out
)

$ErrorActionPreference = 'Stop'
[Console]::InputEncoding = [Text.Encoding]::UTF8
$raw = [Console]::In.ReadToEnd().Trim()

$payloadJson = try { $null = $raw | ConvertFrom-Json -Depth 50; $raw } catch { $raw | ConvertTo-Json -Compress }
if (-not $raw) { $payloadJson = 'null' }

$ts = (Get-Date).ToUniversalTime().ToString('o')
$nonce = $null
if ($Event -eq 'SessionStart') {
    $nonce = 'SUPRAMAN-NONCE-' + -join ((1..12) | ForEach-Object { '{0:x}' -f (Get-Random -Maximum 16) })
}

$head = '{"ts":' + ($ts | ConvertTo-Json) + ',"event":' + ($Event | ConvertTo-Json)
if ($nonce) { $head += ',"nonce":' + ($nonce | ConvertTo-Json) }
$line = $head + ',"payload":' + ($payloadJson -replace "`r?`n", ' ') + '}'

# Hooks can run concurrently; retry the append if another hook holds the file.
for ($i = 0; $i -lt 20; $i++) {
    try { [IO.File]::AppendAllText($Out, $line + "`n", [Text.UTF8Encoding]::new($false)); break }
    catch { Start-Sleep -Milliseconds 50 }
}

if ($nonce) {
    $source = try { ($raw | ConvertFrom-Json).source } catch { 'unknown' }
    Write-Output "$nonce (injected by the SessionStart hook, source=$source). If asked for the SUPRAMAN nonce, repeat this exact value."
}
exit 0

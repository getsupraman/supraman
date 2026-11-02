# Fails unless README.md has the required sections and every relative link and image in it resolves to a file in the repo.
param([string]$Readme = (Join-Path $PSScriptRoot '..\README.md'))
$ErrorActionPreference = 'Stop'

$readme = (Resolve-Path $Readme).Path
$root = Split-Path $readme -Parent
$text = Get-Content -Raw -LiteralPath $readme
$problems = @()

$headings = [regex]::Matches($text, '(?m)^#{1,6}\s+(.+?)\s*$') | ForEach-Object { $_.Groups[1].Value }
foreach ($section in 'Install', 'First steps', 'Build', 'Roadmap', 'Licence') {
    if (-not ($headings | Where-Object { $_ -match "^$([regex]::Escape($section))\b" })) {
        $problems += "no section '$section'"
    }
}

$targets = @()
$targets += [regex]::Matches($text, '!?\[[^\]]*\]\(\s*<?([^)\s>]+)>?(?:\s+"[^"]*")?\s*\)') | ForEach-Object { $_.Groups[1].Value }
$targets += [regex]::Matches($text, '(?i)\b(?:src|srcset|href)\s*=\s*"([^"]+)"') | ForEach-Object { $_.Groups[1].Value }
$checked = 0
foreach ($target in $targets) {
    if ($target -match '^(?i)[a-z][a-z0-9+.-]*:' -or $target.StartsWith('#')) { continue }
    $path = ($target -split '#', 2)[0]
    $path = [uri]::UnescapeDataString($path)
    $full = [IO.Path]::GetFullPath((Join-Path $root $path))
    $checked++
    if (-not $full.StartsWith($root, [StringComparison]::OrdinalIgnoreCase)) {
        $problems += "link outside the repo: $target"
    } elseif (-not (Test-Path -LiteralPath $full -PathType Leaf)) {
        $problems += "missing file: $target"
    }
}

if ($problems.Count -gt 0) {
    $problems | ForEach-Object { Write-Output "FAIL $_" }
    exit 1
}
Write-Output "README.md OK: $($headings.Count) headings, $checked relative links and images resolve"

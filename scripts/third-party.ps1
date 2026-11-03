<#
.SYNOPSIS
    Writes THIRD_PARTY.md, and a release's licence files, from cargo metadata plus the hand-kept assets below.
.DESCRIPTION
    The crates are every package the shipped binaries (supraman-app's supraman.exe, sm's sm.exe and smw.exe)
    link for x86_64-pc-windows-msvc: normal dependencies, transitively, without dev- and build-dependencies.
    Each crate's licence files are the LICENSE*/LICENCE*/COPYING*/NOTICE*/COPYRIGHT*/UNLICENSE* files in its
    package folder (a git checkout's symlink placeholder is followed); a crate that ships none gets the
    standard text from scripts\licenses with its authors as the copyright line.
.PARAMETER Check
    Write nothing to the repo; fail when the committed THIRD_PARTY.md differs from the generated one.
.PARAMETER LicensesDir
    Also write the licence files there: Supraman's LICENSE, the assets' licences and rust-crates.txt
    (each distinct crate licence text once, under the crates that ship it).
#>
param(
    [switch]$Check,
    [string]$LicensesDir
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$platform = 'x86_64-pc-windows-msvc'
$roots = 'supraman-app', 'sm'
$noticeName = '^(LICEN[CS]E|COPYING|NOTICE|COPYRIGHT|UNLICENSE)([-_.].*)?$'

function Read-Text([string]$path) {
    $text = [System.IO.File]::ReadAllText($path) -replace "`r`n", "`n"
    $text.TrimEnd() + "`n"
}

function Write-Text([string]$path, [string]$text) {
    [System.IO.File]::WriteAllText($path, $text, [System.Text.UTF8Encoding]::new($false))
}

# The hand-kept material: not cargo packages, so cargo metadata cannot list it.
$assets = @(
    [pscustomobject]@{
        Title = 'Zed''s terminal crate'
        Text = @'
- Source: https://github.com/zed-industries/zed, `crates/terminal` @ 03efd49220ba06703d04f9c4732fdf814f9c523c
- In Supraman: `crates/supraman-term`, vendored with changes, compiled into `supraman.exe`
- Licence: GPL-3.0-or-later, © Zed Industries, Inc. Full text: `crates/supraman-term/LICENSE-GPL`, in a release
  `licenses\supraman.LICENSE` (the same GPL-3.0 text)
'@
        Files = @()
    }
    [pscustomobject]@{
        Title = 'VS Code Dark Modern for Zed'
        Text = @'
- Source: https://github.com/kevcamel/vscode_dark_modern.zed, `themes/vscode-dark-modern.json` (commit of 2026-09-06)
- In Supraman: `crates/supraman-ui/assets/themes/vscode-dark-modern.json`, compiled into `supraman.exe` as the default theme
- Licence: MIT, © 2024 Kevin Camellini. Full text: `crates/supraman-ui/assets/themes/vscode-dark-modern.LICENSE`,
  in a release `licenses\vscode-dark-modern.LICENSE`

Supraman reads the file unchanged. It adjusts one key when it loads the file, so that the contrast floors hold
(`supraman_ui::themes::fit_floors`): `info` `#BFBFBF66` becomes `#868686`.
'@
        Files = @(@{ From = 'crates/supraman-ui/assets/themes/vscode-dark-modern.LICENSE'; To = 'vscode-dark-modern.LICENSE' })
    }
    [pscustomobject]@{
        Title = 'Lucide icons'
        Text = @'
- Source: Zed's `assets/icons` @ 03efd49220ba06703d04f9c4732fdf814f9c523c, which are Lucide's (https://lucide.dev)
- In Supraman: `crates/supraman-ui/assets/icons/*.svg`, compiled into `supraman.exe`
- Licence: ISC, © Lucide Contributors 2022; portions MIT, © Cole Bemis 2013-2022 (Feather). Full text:
  `crates/supraman-ui/assets/icons/LICENSES`, in a release `licenses\lucide-icons.LICENSE`
'@
        Files = @(@{ From = 'crates/supraman-ui/assets/icons/LICENSES'; To = 'lucide-icons.LICENSE' })
    }
    [pscustomobject]@{
        Title = 'Archivo'
        Text = @'
- Source: the Supraman brand kit v1 (`crates/supraman-app/assets/brand/NOTICE.md`)
- In Supraman: the wordmarks and lockups in `crates/supraman-app/assets/brand` are Archivo outlined to paths,
  compiled into `supraman.exe`
- Licence: SIL Open Font License 1.1, © 2020 The Archivo Project Authors. Full text:
  `crates/supraman-app/assets/brand/source/font/OFL.txt`, in a release `licenses\archivo-OFL.txt`
'@
        Files = @(@{ From = 'crates/supraman-app/assets/brand/source/font/OFL.txt'; To = 'archivo-OFL.txt' })
    }
    [pscustomobject]@{
        Title = 'The Rust standard library'
        Text = @'
- Source: the Rust toolchain of `rust-toolchain.toml`, linked into every exe
- Licence: MIT OR Apache-2.0, © The Rust Project Contributors. Full text: in a release `licenses\rust-crates.txt`
'@
        Files = @()
    }
)

$json = cargo metadata --format-version 1 --locked --filter-platform $platform --manifest-path (Join-Path $repo 'Cargo.toml')
if ($LASTEXITCODE -ne 0) { throw "cargo metadata failed ($LASTEXITCODE)" }
$meta = ($json -join "`n") | ConvertFrom-Json -Depth 100

$packages = @{}
foreach ($p in $meta.packages) { $packages[$p.id] = $p }
$nodes = @{}
foreach ($n in $meta.resolve.nodes) { $nodes[$n.id] = $n }

$queue = [System.Collections.Generic.Queue[string]]::new()
foreach ($root in $roots) {
    $id = ($meta.packages | Where-Object { $_.name -eq $root -and -not $_.source }).id
    if (-not $id) { throw "workspace crate $root not found" }
    $queue.Enqueue($id)
}
$shipped = @{}
while ($queue.Count -gt 0) {
    $id = $queue.Dequeue()
    if ($shipped.ContainsKey($id)) { continue }
    $shipped[$id] = $true
    foreach ($dep in $nodes[$id].deps) {
        if ($dep.dep_kinds | Where-Object { $null -eq $_.kind }) { $queue.Enqueue($dep.pkg) }
    }
}

function Source-Of($p) {
    if ($p.source -like 'registry+*') { return 'crates.io' }
    if ($p.source -match '^git\+([^?#]+).*#([0-9a-f]+)$') { return '{0} @ {1}' -f $Matches[1], $Matches[2].Substring(0, 12) }
    throw "$($p.name) $($p.version): unknown source $($p.source)"
}

function Licence-Texts($p) {
    $dir = Split-Path -Parent $p.manifest_path
    $files = @(Get-ChildItem -File $dir | Where-Object { $_.Name -match $noticeName } | Sort-Object Name)
    if ($p.license_file) {
        $extra = Get-Item (Join-Path $dir $p.license_file)
        if ($files.FullName -notcontains $extra.FullName) { $files += $extra }
    }
    $texts = foreach ($f in $files) {
        $path = $f.FullName
        if ($f.Length -lt 200) {
            $link = (Get-Content -Raw $path).Trim()
            if ($link -notmatch "`n" -and $link -match '^[\w./-]+$' -and (Test-Path -PathType Leaf (Join-Path $f.DirectoryName $link))) {
                $path = Join-Path $f.DirectoryName $link
            }
        }
        [pscustomobject]@{ Name = $f.Name; Text = Read-Text $path }
    }
    if ($texts) { return $texts }

    $authors = @($p.authors | ForEach-Object { $_ -replace '\s*<[^>]*>', '' })
    $holder = if ($authors) { $authors -join ', ' } else { "the $($p.name) authors" }
    foreach ($spdx in 'MIT', 'Apache-2.0') {
        if ($p.license -match "(^|[\s/(])$([regex]::Escape($spdx))($|[\s/)])") {
            $text = Read-Text (Join-Path $PSScriptRoot "licenses\$spdx.txt")
            $text = $text.Replace('{copyright}', "Copyright (c) $holder")
            return [pscustomobject]@{ Name = "$spdx (standard text; the package ships no licence file)"; Text = $text }
        }
    }
    throw "$($p.name) $($p.version) ships no licence file and scripts\licenses has no text for '$($p.license)'"
}

$crates = [System.Collections.Generic.List[object]]@($shipped.Keys | ForEach-Object { $packages[$_] } | Where-Object { $_.source })
$crates.Sort({ param($a, $b) [string]::CompareOrdinal("$($a.name) $($a.version)", "$($b.name) $($b.version)") })

$md = [System.Text.StringBuilder]::new()
[void]$md.Append(@"
# Third-party material shipped with Supraman

Supraman is free software under the GNU General Public License, version 3 or (at your option) any later version
(``LICENSE``). Its binaries contain the material below, each part under its own licence.

Generated by ``scripts/third-party.ps1`` from ``cargo metadata`` and the hand-kept entries in that script; do not edit
by hand. ``pwsh -File scripts/third-party.ps1 -Check`` fails when this file is stale.

## Bundled assets and vendored code

"@ + "`n")
foreach ($a in $assets) {
    [void]$md.Append("### $($a.Title)`n`n$($a.Text.Replace("`r`n", "`n").TrimEnd())`n`n")
}
[void]$md.Append(@"
## Rust crates

The $($crates.Count) crates ``supraman.exe``, ``sm.exe`` and ``smw.exe`` are built from ($platform, without dev- and
build-dependencies). Their licence texts and copyright notices, as each package ships them, are in a release's
``licenses\rust-crates.txt``.

| Crate | Version | Licence | Source |
|---|---|---|---|

"@)
foreach ($p in $crates) {
    $licence = if ($p.license) { $p.license } else { "see $($p.license_file)" }
    [void]$md.Append("| $($p.name) | $($p.version) | $licence | $(Source-Of $p) |`n")
}
[void]$md.Append(@"

## In a release

``licenses\`` holds ``supraman.LICENSE`` (GPL-3.0), ``rust-crates.txt``, ``vscode-dark-modern.LICENSE``,
``lucide-icons.LICENSE`` and ``archivo-OFL.txt``.

"@)
$generated = $md.ToString().Replace("`r`n", "`n")

$thirdParty = Join-Path $repo 'THIRD_PARTY.md'
if ($Check) {
    $committed = if (Test-Path $thirdParty) { Read-Text $thirdParty } else { '' }
    if ($committed -ne $generated) {
        $diff = Compare-Object ($committed -split "`n") ($generated -split "`n") | Select-Object -First 10 |
            ForEach-Object { '{0} {1}' -f $(if ($_.SideIndicator -eq '=>') { '+' } else { '-' }), $_.InputObject }
        throw "THIRD_PARTY.md is stale; run pwsh -File scripts/third-party.ps1 and commit it:`n$($diff -join "`n")"
    }
    Write-Output "THIRD_PARTY.md is current ($($crates.Count) crates)"
}
else {
    Write-Text $thirdParty $generated
    Write-Output "THIRD_PARTY.md written ($($crates.Count) crates)"
}

if ($LicensesDir) {
    New-Item -ItemType Directory -Force $LicensesDir | Out-Null
    Copy-Item (Join-Path $repo 'LICENSE') (Join-Path $LicensesDir 'supraman.LICENSE')
    foreach ($a in $assets) {
        foreach ($f in $a.Files) { Copy-Item (Join-Path $repo $f.From) (Join-Path $LicensesDir $f.To) }
    }

    $groups = [ordered]@{}
    $std = (Read-Text (Join-Path $PSScriptRoot 'licenses\MIT.txt')).Replace('{copyright}', 'Copyright (c) The Rust Project Contributors')
    $groups['std'] = [pscustomobject]@{ Users = [System.Collections.Generic.List[string]]@('The Rust standard library (MIT OR Apache-2.0; MIT shown)'); Text = $std }
    $sha = [System.Security.Cryptography.SHA256]::Create()
    foreach ($p in $crates) {
        foreach ($t in Licence-Texts $p) {
            $key = [Convert]::ToHexString($sha.ComputeHash([System.Text.Encoding]::UTF8.GetBytes($t.Text)))
            if (-not $groups.Contains($key)) {
                $groups[$key] = [pscustomobject]@{ Users = [System.Collections.Generic.List[string]]::new(); Text = $t.Text }
            }
            $groups[$key].Users.Add("$($p.name) $($p.version) ($($t.Name))")
        }
    }
    $rule = '=' * 80
    $out = [System.Text.StringBuilder]::new()
    [void]$out.Append("Licence texts and copyright notices of the Rust code in supraman.exe, sm.exe and smw.exe.`n")
    [void]$out.Append("Each text is given once, after the list of crates that ship it (THIRD_PARTY.md lists every crate).`n")
    foreach ($g in $groups.Values) {
        [void]$out.Append("`n$rule`n")
        foreach ($u in $g.Users) { [void]$out.Append("$u`n") }
        [void]$out.Append("$('-' * 80)`n`n$($g.Text)")
    }
    Write-Text (Join-Path $LicensesDir 'rust-crates.txt') $out.ToString()
    Write-Output "licences written to $LicensesDir ($($groups.Count) distinct texts)"
}

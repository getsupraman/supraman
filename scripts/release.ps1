<#
.SYNOPSIS
    Builds a Supraman release folder and zip that run from any path (docs/RELEASE.md).
.DESCRIPTION
    cargo build --release --workspace, then assembles

        <OutDir>\supraman.exe
        <OutDir>\supraman.pdb                (names the frames of a panic's backtrace in foreman.log)
        <OutDir>\sm.exe, smw.exe
        <OutDir>\fm.exe, fmw.exe               (copies of sm.exe and smw.exe for one release, docs/RELEASE.md)
        <OutDir>\plugins\supraman\**          (hooks, roles, skills)
        <OutDir>\plugins\supraman\bin\sm.exe, fm.exe  (the hooks' copies, via sm plugin install)
        <OutDir>\VERSION
        <OutDir>\THIRD_PARTY.md
        <OutDir>\licenses\*.LICENSE           (third-party licences, THIRD_PARTY.md)

    and zips it to <OutDir>\..\Supraman-<version>.zip. Honours an existing CARGO_TARGET_DIR.
.PARAMETER OutDir
    The release folder. Default: <target dir>\dist\Supraman, where <target dir> is CARGO_TARGET_DIR or
    <repo>\target. An existing folder is replaced only if it holds a VERSION file (an earlier release).
#>
param(
    [string]$OutDir
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$target = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $repo 'target' }
if (-not $OutDir) { $OutDir = Join-Path $target 'dist\Supraman' }
$OutDir = [System.IO.Path]::GetFullPath($OutDir)

Push-Location $repo
try {
    cargo build --release --workspace
    if ($LASTEXITCODE -ne 0) { throw "cargo build --release --workspace failed ($LASTEXITCODE)" }

    $built = Join-Path $target 'release'
    foreach ($exe in 'supraman.exe', 'supraman.pdb', 'sm.exe', 'smw.exe') {
        if (-not (Test-Path (Join-Path $built $exe))) { throw "$exe is missing from $built" }
    }

    if (Test-Path $OutDir) {
        $entries = @(Get-ChildItem -Force $OutDir)
        if ($entries.Count -gt 0 -and -not (Test-Path (Join-Path $OutDir 'VERSION'))) {
            throw "$OutDir exists and is not an earlier release (no VERSION): not replacing it"
        }
        Remove-Item -Recurse -Force $OutDir
    }
    $plugin = Join-Path $OutDir 'plugins\supraman'
    New-Item -ItemType Directory -Force $plugin | Out-Null
    Copy-Item (Join-Path $built 'supraman.exe'), (Join-Path $built 'supraman.pdb'), (Join-Path $built 'sm.exe'), (Join-Path $built 'smw.exe') $OutDir
    Copy-Item (Join-Path $built 'sm.exe') (Join-Path $OutDir 'fm.exe')
    Copy-Item (Join-Path $built 'smw.exe') (Join-Path $OutDir 'fmw.exe')
    # bin/ is the dev tree's own hooks copy (git-ignored); the release gets a fresh one below.
    Get-ChildItem (Join-Path $repo 'plugins\supraman') | Where-Object Name -ne 'bin' |
        Copy-Item -Destination $plugin -Recurse
    & (Join-Path $OutDir 'sm.exe') plugin install $plugin
    if ($LASTEXITCODE -ne 0) { throw "sm plugin install $plugin failed ($LASTEXITCODE)" }

    Copy-Item (Join-Path $repo 'THIRD_PARTY.md') $OutDir
    $licenses = Join-Path $OutDir 'licenses'
    New-Item -ItemType Directory -Force $licenses | Out-Null
    Copy-Item (Join-Path $repo 'crates/supraman-ui/assets/themes/*.LICENSE') $licenses

    $cli = Get-Content -Raw (Join-Path $repo 'crates\supraman-claude\src\cli.rs')
    $triple = {
        param($name)
        $m = [regex]::Match($cli, "pub const ${name}: \(u32, u32, u32\) = \((\d+), (\d+), (\d+)\);")
        if (-not $m.Success) { throw "cannot read $name from crates\supraman-claude\src\cli.rs" }
        '{0}.{1}.{2}' -f $m.Groups[1].Value, $m.Groups[2].Value, $m.Groups[3].Value
    }
    $minimum = & $triple 'MIN_VERSION'
    $tested = & $triple 'TESTED_VERSION'
    $sha = (git rev-parse --short=12 HEAD).Trim()
    if (git status --porcelain) { $sha = "$sha-dirty" }
    $crate = [regex]::Match((Get-Content -Raw (Join-Path $repo 'crates\supraman-app\Cargo.toml')), '(?m)^version = "([^"]+)"').Groups[1].Value
    $now = (Get-Date).ToUniversalTime()
    $version = '{0}-{1}-{2}' -f $crate, $now.ToString('yyyyMMdd'), $sha
    @(
        "version $version"
        "commit $sha"
        "built $($now.ToString('yyyy-MM-ddTHH:mm:ssZ'))"
        "claude tested $tested"
        "claude minimum $minimum"
    ) | Set-Content -Encoding utf8 (Join-Path $OutDir 'VERSION')

    $zip = Join-Path (Split-Path -Parent $OutDir) "Supraman-$version.zip"
    if (Test-Path $zip) { Remove-Item -Force $zip }
    Compress-Archive -Path $OutDir -DestinationPath $zip
    Write-Output "release $OutDir"
    Write-Output "zip $zip"
}
finally {
    Pop-Location
}

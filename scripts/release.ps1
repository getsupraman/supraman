<#
.SYNOPSIS
    Builds a Foreman release folder and zip that run from any path (docs/RELEASE.md).
.DESCRIPTION
    cargo build --release --workspace, then assembles

        <OutDir>\foreman.exe
        <OutDir>\foreman.pdb                 (names the frames of a panic's backtrace in foreman.log)
        <OutDir>\fm.exe
        <OutDir>\plugins\foreman\**          (hooks, roles, skills)
        <OutDir>\plugins\foreman\bin\fm.exe  (the hooks' copy, via fm plugin install)
        <OutDir>\VERSION
        <OutDir>\THIRD_PARTY.md
        <OutDir>\licenses\*.LICENSE           (third-party licences, THIRD_PARTY.md)

    and zips it to <OutDir>\..\Foreman-<version>.zip. Honours an existing CARGO_TARGET_DIR.
.PARAMETER OutDir
    The release folder. Default: <target dir>\dist\Foreman, where <target dir> is CARGO_TARGET_DIR or
    <repo>\target. An existing folder is replaced only if it holds a VERSION file (an earlier release).
#>
param(
    [string]$OutDir
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$target = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $repo 'target' }
if (-not $OutDir) { $OutDir = Join-Path $target 'dist\Foreman' }
$OutDir = [System.IO.Path]::GetFullPath($OutDir)

Push-Location $repo
try {
    cargo build --release --workspace
    if ($LASTEXITCODE -ne 0) { throw "cargo build --release --workspace failed ($LASTEXITCODE)" }

    $built = Join-Path $target 'release'
    foreach ($exe in 'foreman.exe', 'foreman.pdb', 'fm.exe', 'fmw.exe') {
        if (-not (Test-Path (Join-Path $built $exe))) { throw "$exe is missing from $built" }
    }

    if (Test-Path $OutDir) {
        $entries = @(Get-ChildItem -Force $OutDir)
        if ($entries.Count -gt 0 -and -not (Test-Path (Join-Path $OutDir 'VERSION'))) {
            throw "$OutDir exists and is not an earlier release (no VERSION): not replacing it"
        }
        Remove-Item -Recurse -Force $OutDir
    }
    $plugin = Join-Path $OutDir 'plugins\foreman'
    New-Item -ItemType Directory -Force $plugin | Out-Null
    Copy-Item (Join-Path $built 'foreman.exe'), (Join-Path $built 'foreman.pdb'), (Join-Path $built 'fm.exe'), (Join-Path $built 'fmw.exe') $OutDir
    # bin/ is the dev tree's own hooks copy (git-ignored); the release gets a fresh one below.
    Get-ChildItem (Join-Path $repo 'plugins\foreman') | Where-Object Name -ne 'bin' |
        Copy-Item -Destination $plugin -Recurse
    & (Join-Path $OutDir 'fm.exe') plugin install $plugin
    if ($LASTEXITCODE -ne 0) { throw "fm plugin install $plugin failed ($LASTEXITCODE)" }

    Copy-Item (Join-Path $repo 'THIRD_PARTY.md') $OutDir
    $licenses = Join-Path $OutDir 'licenses'
    New-Item -ItemType Directory -Force $licenses | Out-Null
    Copy-Item (Join-Path $repo 'crates/foreman-ui/assets/themes/*.LICENSE') $licenses

    $cli = Get-Content -Raw (Join-Path $repo 'crates\foreman-claude\src\cli.rs')
    $triple = {
        param($name)
        $m = [regex]::Match($cli, "pub const ${name}: \(u32, u32, u32\) = \((\d+), (\d+), (\d+)\);")
        if (-not $m.Success) { throw "cannot read $name from crates\foreman-claude\src\cli.rs" }
        '{0}.{1}.{2}' -f $m.Groups[1].Value, $m.Groups[2].Value, $m.Groups[3].Value
    }
    $minimum = & $triple 'MIN_VERSION'
    $tested = & $triple 'TESTED_VERSION'
    $sha = (git rev-parse --short=12 HEAD).Trim()
    if (git status --porcelain) { $sha = "$sha-dirty" }
    $crate = [regex]::Match((Get-Content -Raw (Join-Path $repo 'crates\foreman-app\Cargo.toml')), '(?m)^version = "([^"]+)"').Groups[1].Value
    $now = (Get-Date).ToUniversalTime()
    $version = '{0}-{1}-{2}' -f $crate, $now.ToString('yyyyMMdd'), $sha
    @(
        "version $version"
        "commit $sha"
        "built $($now.ToString('yyyy-MM-ddTHH:mm:ssZ'))"
        "claude tested $tested"
        "claude minimum $minimum"
    ) | Set-Content -Encoding utf8 (Join-Path $OutDir 'VERSION')

    $zip = Join-Path (Split-Path -Parent $OutDir) "Foreman-$version.zip"
    if (Test-Path $zip) { Remove-Item -Force $zip }
    Compress-Archive -Path $OutDir -DestinationPath $zip
    Write-Output "release $OutDir"
    Write-Output "zip $zip"
}
finally {
    Pop-Location
}

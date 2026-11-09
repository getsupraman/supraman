<#
.SYNOPSIS
    Prints the `-p <crate>` arguments, one per line, for the workspace crates a diff touches and every workspace crate depending on them.
.DESCRIPTION
    The diff is <Base> against the working tree, untracked files included, or against <Head> when given. A change at the root (Cargo.toml,
    Cargo.lock, .cargo\, rust-toolchain*) means every member; a change outside every crate (docs, scripts) none.
    Dependents follow normal, dev and build dependencies on workspace members (cargo metadata --offline).
      powershell -File scripts/heavy-lock.ps1 cargo test --profile unit $(powershell -File scripts/affected.ps1)
.PARAMETER Base
    The revision to diff against. Default: the merge base of HEAD and master.
.PARAMETER Head
    A revision to diff to instead of the working tree (a commit's own change: -Base <sha>~1 -Head <sha>).
#>
param([string]$Base, [string]$Head)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
Push-Location $repo
try {
    if (-not $Base) {
        $Base = git merge-base HEAD master
        if ($LASTEXITCODE -ne 0 -or -not $Base) { throw 'no merge base with master: pass -Base <rev>' }
    }
    if ($Head) {
        $changed = @(git diff --name-only $Base $Head --)
        if ($LASTEXITCODE -ne 0) { throw "git diff $Base $Head failed" }
    } else {
        $changed = @(git diff --name-only $Base --)
        if ($LASTEXITCODE -ne 0) { throw "git diff $Base failed" }
        $changed += @(git ls-files --others --exclude-standard)
    }

    $json = cargo metadata --no-deps --format-version 1 --offline
    if ($LASTEXITCODE -ne 0 -or -not $json) { throw 'cargo metadata failed' }
    $meta = $json | ConvertFrom-Json
    $root = [IO.Path]::GetFullPath($meta.workspace_root).TrimEnd('\')
    $members = @($meta.packages | Where-Object { $meta.workspace_members -contains $_.id })
    $dirs = @{}
    foreach ($p in $members) {
        $dir = Split-Path -Parent ([IO.Path]::GetFullPath($p.manifest_path))
        $dirs[$p.name] = $dir.Substring($root.Length).TrimStart('\').Replace('\', '/') + '/'
    }

    $touched = @{}
    $all = $false
    foreach ($file in $changed | Where-Object { $_ } | Sort-Object -Unique) {
        if ($file -match '^(Cargo\.toml|Cargo\.lock|rust-toolchain(\.toml)?|\.cargo/.*)$') { $all = $true; break }
        foreach ($name in $dirs.Keys) {
            if ($file.StartsWith($dirs[$name], [StringComparison]::OrdinalIgnoreCase)) { $touched[$name] = $true }
        }
    }

    $names = @($members | ForEach-Object { $_.name })
    if ($all) {
        $affected = $names
    } else {
        $dependents = @{}
        foreach ($p in $members) {
            foreach ($d in $p.dependencies) {
                if ($d.path -and $names -contains $d.name) {
                    if (-not $dependents[$d.name]) { $dependents[$d.name] = @() }
                    $dependents[$d.name] += $p.name
                }
            }
        }
        $queue = [Collections.Queue]::new(@($touched.Keys))
        while ($queue.Count -gt 0) {
            $name = $queue.Dequeue()
            foreach ($up in @($dependents[$name])) {
                if ($up -and -not $touched[$up]) { $touched[$up] = $true; $queue.Enqueue($up) }
            }
        }
        $affected = @($names | Where-Object { $touched[$_] })
    }
    if ($affected.Count -eq 0) { [Console]::Error.WriteLine("affected: no workspace crate changed since $Base") }
    foreach ($name in $affected) { '-p'; $name }
}
finally { Pop-Location }

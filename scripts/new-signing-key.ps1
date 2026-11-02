<#
.SYNOPSIS
    Makes the release signing key pair once (docs/RELEASE.md, "Signing"). René runs it; no agent does.
.DESCRIPTION
    Builds sm, then runs

        sm release keygen --public-out crates\supraman-core\src\update.pub | gh secret set UPDATE_SIGNING_KEY --repo <Repo>

    The secret key exists only in that pipe and in the repository secret: never on disk, never on the screen
    (sm refuses a console stdout). The public key replaces update.pub, which you then commit.
.PARAMETER Repo
    The GitHub repository that holds the secret. Default: getsupraman/supraman.
#>
param(
    [string]$Repo = 'getsupraman/supraman'
)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
$target = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $repoRoot 'target' }
$public = Join-Path $repoRoot 'crates\supraman-core\src\update.pub'

Push-Location $repoRoot
try {
    gh auth status *> $null
    if ($LASTEXITCODE -ne 0) { throw 'gh is not signed in: run gh auth login first' }
    cargo build --release -p sm
    if ($LASTEXITCODE -ne 0) { throw "cargo build --release -p sm failed ($LASTEXITCODE)" }
    $sm = Join-Path $target 'release\sm.exe'

    $before = Get-Content -Raw $public
    & $sm release keygen --public-out $public | gh secret set UPDATE_SIGNING_KEY --repo $Repo
    $gh = $LASTEXITCODE
    $after = Get-Content -Raw $public
    if ($after -eq $before) { throw 'sm release keygen did not write update.pub: nothing was changed' }
    if ($gh -ne 0) {
        Set-Content -NoNewline -Encoding utf8 $public $before
        throw "gh secret set failed ($gh): update.pub is restored, the new key is gone; run the script again"
    }
    Write-Output "UPDATE_SIGNING_KEY is set on $Repo and $public holds its public key."
    Write-Output 'Commit it: git add crates/supraman-core/src/update.pub; git commit -m "release signing key"'
}
finally {
    Pop-Location
}

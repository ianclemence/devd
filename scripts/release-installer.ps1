# Builds the Windows installer.
#
# The installer is per-user: it needs no administrator rights, installs under
# %LOCALAPPDATA%\Programs\devd, and writes its shortcuts and uninstall entry to
# the current user's registry. The first start runs `devd setup`, which
# downloads PHP, nginx and MariaDB and creates a site.
#
# Usage:
#   .\scripts\release-installer.ps1
#   .\scripts\release-installer.ps1 -Nsis "C:\Program Files (x86)\NSIS\makensis.exe"
#
# Requires NSIS 3 (makensis). The release binary is built first.

[CmdletBinding()]
param(
    [string]$Nsis
)

$ErrorActionPreference = "Continue"
$repo = Split-Path -Parent $PSScriptRoot
Set-Location $repo

if (-not $Nsis) {
    $found = Get-Command makensis -ErrorAction SilentlyContinue
    if ($found) { $Nsis = $found.Source }
}
if (-not $Nsis -or -not (Test-Path $Nsis)) {
    throw "makensis was not found. Install NSIS 3 (https://nsis.sourceforge.io) or pass -Nsis <path to makensis.exe>."
}

$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.*)"' |
    Select-Object -First 1).Matches[0].Groups[1].Value
if (-not $version) { throw "could not read the version from Cargo.toml" }

Write-Host "==> cargo build --release" -ForegroundColor Cyan
cargo build --release -p devd-cli -p devd-desktop
if ($LASTEXITCODE -ne 0) { throw "the release build failed" }

$source = Join-Path $repo "target\release\devd.exe"
if (-not (Test-Path $source)) { throw "the build did not produce $source" }
$desktop = Join-Path $repo "target\release\devd-desktop.exe"
if (-not (Test-Path $desktop)) { throw "the build did not produce $desktop" }

New-Item -ItemType Directory -Force dist | Out-Null
$out = Join-Path $repo "dist\devd-setup-$version.exe"

Write-Host "==> makensis" -ForegroundColor Cyan
& $Nsis "/DVERSION=$version" "/DSOURCE=$source" "/DDESKTOP=$desktop" "/DOUTFILE=$out" "/DROOT=$repo" "installer\devd.nsi"
if ($LASTEXITCODE -ne 0) { throw "makensis failed" }
if (-not (Test-Path $out)) { throw "the installer was not produced at $out" }

$sha = (Get-FileHash $out -Algorithm SHA256).Hash.ToLower()
"$sha  devd-setup-$version.exe" | Out-File -Encoding ascii "dist\SHA256SUMS.txt"

Write-Host ""
Write-Host "wrote dist\devd-setup-$version.exe" -ForegroundColor Green
Get-Content "dist\SHA256SUMS.txt"

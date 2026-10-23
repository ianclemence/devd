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
$webview2 = Join-Path $repo "target\release\WebView2Loader.dll"
if (-not (Test-Path $webview2)) { throw "the build did not produce $webview2" }

New-Item -ItemType Directory -Force dist | Out-Null
$out = Join-Path $repo "dist\devd-setup-$version.exe"

Write-Host "==> makensis" -ForegroundColor Cyan
& $Nsis "/DVERSION=$version" "/DSOURCE=$source" "/DDESKTOP=$desktop" "/DWEBVIEW2=$webview2" "/DOUTFILE=$out" "/DROOT=$repo" "installer\devd.nsi"
if ($LASTEXITCODE -ne 0) { throw "makensis failed" }
if (-not (Test-Path $out)) { throw "the installer was not produced at $out" }

$sha = (Get-FileHash $out -Algorithm SHA256).Hash.ToLower()
$name = "devd-setup-$version.exe"

# The checksum file lists every artifact. The installer is added to it rather
# than replacing it, so a release always has one complete list.
$sums = Join-Path $repo "dist\SHA256SUMS.txt"
$lines = @()
if (Test-Path $sums) {
    $lines = @(Get-Content $sums | Where-Object { $_ -and $_ -notmatch [regex]::Escape($name) })
}
$lines += "$sha  $name"
$lines | Out-File -Encoding ascii $sums

Write-Host ""
Write-Host "wrote dist\$name" -ForegroundColor Green
Get-Content $sums

# Builds the Linux artifact from Windows, using a Linux environment on this
# machine: a WSL distribution or a Docker Linux container.
#
# The build inside that environment is scripts/release-linux.sh, which runs the
# test suite, builds the binary, exercises it, and writes dist/.
#
# Usage:
#   .\scripts\release-linux.ps1                  # first WSL distro, or Docker
#   .\scripts\release-linux.ps1 -Distro Ubuntu
#   .\scripts\release-linux.ps1 -DockerImage rust:1-bookworm
#
# It exits non-zero when neither environment exists. In that case run
# scripts/release-linux.sh on any Linux machine and copy dist/ back.

[CmdletBinding()]
param(
    [string]$Distro,
    [string]$DockerImage = "rust:1-bookworm"
)

$ErrorActionPreference = "Stop"
$repo = Split-Path -Parent $PSScriptRoot

function ConvertTo-WslPath([string]$path) {
    $drive = $path.Substring(0, 1).ToLowerInvariant()
    $rest = $path.Substring(2).Replace('\', '/')
    return "/mnt/$drive$rest"
}

function Get-WslDistros {
    $raw = & wsl -l -q 2>$null
    if ($LASTEXITCODE -ne 0) { return @() }
    return @($raw | ForEach-Object { ($_ -replace "`0", "").Trim() } | Where-Object { $_ })
}

$distros = if ($Distro) { @($Distro) } else { Get-WslDistros }
if ($distros.Count -gt 0) {
    $name = $distros[0]
    $source = ConvertTo-WslPath $repo
    $work = "/tmp/devd-release-build"

    Write-Host "==> building in WSL ($name)" -ForegroundColor Cyan
    # The source is copied into the Linux filesystem: a build over /mnt/d is
    # several times slower, and the copy is a few hundred kilobytes.
    $script = @"
set -eu
rm -rf $work
mkdir -p $work
cd '$source'
tar -cf - --exclude=target --exclude=dist . | tar -xf - -C $work
cd $work
TARGET= sh ./scripts/release-linux.sh
mkdir -p '$source/dist'
cp dist/* '$source/dist/'
"@
    & wsl -d $name -- sh -lc $script
    if ($LASTEXITCODE -ne 0) { throw "the Linux build failed in WSL ($name)" }
    Write-Host "==> dist/" -ForegroundColor Green
    Get-ChildItem (Join-Path $repo "dist") | Select-Object Name, Length
    exit 0
}

$docker = Get-Command docker -ErrorAction SilentlyContinue
if ($docker) {
    Write-Host "==> building in Docker ($DockerImage)" -ForegroundColor Cyan
    & docker run --rm -v "${repo}:/src" -v "${repo}/dist:/out" -w /src $DockerImage `
        sh -c "sh scripts/release-linux.sh && cp dist/* /out/"
    if ($LASTEXITCODE -ne 0) { throw "the Linux build failed in Docker" }
    Write-Host "==> dist/" -ForegroundColor Green
    Get-ChildItem (Join-Path $repo "dist") | Select-Object Name, Length
    exit 0
}

Write-Host "No Linux environment is available on this machine." -ForegroundColor Yellow
Write-Host ""
Write-Host "WSL is installed but has no distribution, and Docker is not on PATH."
Write-Host "If either of those is not true, this script did not see it; check:"
Write-Host "    wsl -l -q"
Write-Host "    docker version"
Write-Host ""
Write-Host "To create one:"
Write-Host "    wsl --install -d Ubuntu      (administrator; a reboot the first time)"
Write-Host "    or install Docker Desktop with Linux containers"
Write-Host ""
Write-Host "Otherwise run scripts/release-linux.sh on any Linux machine and copy"
Write-Host "the resulting dist/ back here."
exit 1

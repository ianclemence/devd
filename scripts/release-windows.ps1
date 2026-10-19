# Builds and packages the Windows binary.
#
# Runs the test suite, builds the release binary, exercises it, and writes the
# artifact and its checksum to dist/.
#
# Usage:
#   .\scripts\release-windows.ps1
#   .\scripts\release-windows.ps1 -SkipTests
#
# Requirements: rustc and cargo on PATH, and a C compiler for the bundled
# SQLite (the repository's .cargo/config.toml covers the GNU toolchain).

[CmdletBinding()]
param(
    [switch]$SkipTests
)

# Native tools write progress to stderr, which PowerShell would otherwise treat
# as an error. Failures are checked explicitly through $LASTEXITCODE instead.
$ErrorActionPreference = "Continue"
$repo = Split-Path -Parent $PSScriptRoot
Set-Location $repo

$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.*)"' |
    Select-Object -First 1).Matches[0].Groups[1].Value
if (-not $version) {
    throw "could not read the version from Cargo.toml"
}

$target = "x86_64-pc-windows-gnu"
$name = "devd-v$version-$target.exe"

Write-Host "==> devd $version for $target" -ForegroundColor Cyan

if (-not $SkipTests) {
    Write-Host "==> cargo test --workspace" -ForegroundColor Cyan
    cargo test --workspace
    if ($LASTEXITCODE -ne 0) { throw "the test suite failed" }
}

Write-Host "==> cargo build --release" -ForegroundColor Cyan
cargo build --release -p devd-cli
if ($LASTEXITCODE -ne 0) { throw "the release build failed" }

$bin = "target/release/devd.exe"
if (-not (Test-Path $bin)) { throw "the build did not produce $bin" }

# The artifact is only published after it has run. A temporary home keeps this
# away from whatever is already installed on the machine.
$smokeHome = Join-Path ([System.IO.Path]::GetTempPath()) ("devd-smoke-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path $smokeHome | Out-Null

try {
    Write-Host "==> smoke test" -ForegroundColor Cyan
    $reported = & $bin --version
    if ($reported -ne "devd $version") {
        throw "expected 'devd $version', got '$reported'"
    }
    Write-Host "    $reported"

    $env:DEVD_HOME = $smokeHome
    & $bin capabilities --json | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "the capability registry could not be listed" }
    Write-Host "    capability registry: ok"

    # Not fatal: a busy port or a missing Git says something about the machine,
    # not about the binary. Printed so a broken build is visible in the log.
    Write-Host "==> devd doctor (informational)" -ForegroundColor Cyan
    & $bin doctor
}
finally {
    Remove-Item Env:DEVD_HOME -ErrorAction SilentlyContinue
    Remove-Item $smokeHome -Recurse -Force -ErrorAction SilentlyContinue
}

New-Item -ItemType Directory -Force dist | Out-Null
Copy-Item $bin "dist/$name" -Force

$sha = (Get-FileHash "dist/$name" -Algorithm SHA256).Hash.ToLower()
"$sha  $name" | Out-File -Encoding ascii "dist/SHA256SUMS.txt"

Write-Host ""
Write-Host "wrote dist/$name" -ForegroundColor Green
Get-Content "dist/SHA256SUMS.txt"

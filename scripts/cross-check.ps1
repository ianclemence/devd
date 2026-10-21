# Type-checks devd for Linux and macOS from a Windows machine.
#
# Code behind `cfg(unix)` and `cfg(target_os = "macos")` is not compiled by a
# Windows build, so a mistake there is invisible until somebody builds on that
# system. This checks it with the real compiler front end. It does not link or
# run anything: it proves the code compiles, not that it behaves.
#
#   .\scripts\cross-check.ps1                  # Linux, then macOS
#   .\scripts\cross-check.ps1 -Targets linux   # one of them
#
# A few dependencies compile C when they build (SQLite, ring). Nothing here needs
# that C to be correct, because nothing is linked, so a stand-in compiler that
# writes empty object files is enough to let `cargo check` get past their build
# scripts. It is built on first use from scripts/stubcc.rs.

param(
    [ValidateSet("linux", "macos")]
    [string[]]$Targets = @("linux", "macos"),
    [string]$TargetDir = "C:\dtx"
)

. "$PSScriptRoot\dev-env.ps1"

$toolchain = "stable-x86_64-pc-windows-gnu"
$triples = @{
    linux = @("x86_64-unknown-linux-gnu")
    macos = @("aarch64-apple-darwin")
}

$stub = Join-Path $TargetDir "stubcc.exe"
New-Item -ItemType Directory -Force $TargetDir | Out-Null
if (-not (Test-Path $stub)) {
    rustc "$PSScriptRoot\stubcc.rs" -o $stub
    if ($LASTEXITCODE -ne 0) { throw "could not build the stand-in C compiler" }
}

$env:CARGO_TARGET_DIR = $TargetDir
$failed = $false
foreach ($target in $Targets) {
    foreach ($triple in $triples[$target]) {
        rustup target add $triple --toolchain $toolchain 2>&1 | Out-Null
        $key = $triple.Replace("-", "_")
        Set-Item "env:CC_$key" $stub
        Set-Item "env:AR_$key" "ar"
        Write-Host "== cargo check for $triple"
        # The desktop shell links the platform's web view and is not part of the
        # logic being checked.
        cargo check --workspace --exclude devd-desktop --all-targets --target $triple
        if ($LASTEXITCODE -ne 0) { $failed = $true }
    }
}
if ($failed) { exit 1 }

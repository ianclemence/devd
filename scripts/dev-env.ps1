# Prepares a PowerShell session to build and test devd on Windows.
#
# devd targets x86_64-pc-windows-gnu (see .cargo/config.toml). Two things on a
# typical machine get in the way:
#
#   * a default `cargo` from the MSVC toolchain sits first on PATH, and its
#     linker step finds Git's `link.exe` instead of Microsoft's;
#   * `target` may be a junction holding artifacts built for another triple,
#     which rustc rejects with E0461.
#
# Dot-source it, then use cargo as usual:
#
#   . .\scripts\dev-env.ps1
#   cargo test --workspace
#
# The GNU toolchain is put first on PATH for this session only, and builds go to
# a short directory outside the checkout: the dependency tree is deep enough to
# overflow Windows path limits under a long scratch path.

param(
    [string]$TargetDir = "C:\dt"
)

$toolchain = "stable-x86_64-pc-windows-gnu"
$rustupHome = if ($env:RUSTUP_HOME) { $env:RUSTUP_HOME } else { Join-Path $env:USERPROFILE ".rustup" }
$bin = Join-Path $rustupHome "toolchains\$toolchain\bin"

if (-not (Test-Path (Join-Path $bin "cargo.exe"))) {
    Write-Error "The $toolchain toolchain is not installed. Run: rustup toolchain install $toolchain"
    return
}

if (-not (($env:PATH -split ";") -contains $bin)) {
    $env:PATH = "$bin;$env:PATH"
}
$env:CARGO_TARGET_DIR = $TargetDir

if (-not (Get-Command gcc -ErrorAction SilentlyContinue)) {
    Write-Warning "No C compiler (gcc) on PATH. The bundled SQLite needs one; install MinGW-w64."
}

$host_triple = (rustc -vV | Select-String "^host:").ToString().Split(" ")[1]
Write-Host "devd build environment: host $host_triple, target dir $TargetDir"

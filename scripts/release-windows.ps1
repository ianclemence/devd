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

# The artifacts are wherever cargo put them.
#
# This project's dependency tree overflows Windows path limits under a long
# checkout path, so dev-env.ps1 points cargo at a short directory such as
# C:\dt. A release must not depend on which shell started it, so the same
# directory is chosen here when the caller has not: without it the build lands
# under a path long enough to crash gcc while compiling ring, and the failure
# reads only as "the release build failed".
if (-not $env:CARGO_TARGET_DIR) {
    $env:CARGO_TARGET_DIR = "C:\dt"
}

Write-Host "==> cargo build --release" -ForegroundColor Cyan
cargo build --release -p devd-cli -p devd-desktop
if ($LASTEXITCODE -ne 0) { throw "the release build failed" }

$targetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $repo "target" }
$bin = Join-Path $targetDir "release/devd.exe"
if (-not (Test-Path $bin)) { throw "the build did not produce $bin" }
$desktopBin = Join-Path $targetDir "release/devd-desktop.exe"
if (-not (Test-Path $desktopBin)) { throw "the build did not produce $desktopBin" }

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
$desktopName = "devd-desktop-v$version-$target.exe"
Copy-Item $desktopBin "dist/$desktopName" -Force
$webview2 = Join-Path $targetDir "release/WebView2Loader.dll"
if (-not (Test-Path $webview2)) { throw "the build did not produce $webview2" }

$zipName = "devd-desktop-v$version-$target.zip"
$staging = Join-Path ([System.IO.Path]::GetTempPath()) ("devd-desktop-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path $staging | Out-Null
try {
    # A portable window is not one file. It needs the WebView2 loader, the
    # command (for `devd setup` and the terminal) and a launcher that runs setup
    # before the first window. Shipping only the window produced an exe that
    # died with a missing-DLL error for anyone who downloaded it alone.
    Copy-Item $desktopBin (Join-Path $staging "devd-desktop.exe")
    Copy-Item $webview2 (Join-Path $staging "WebView2Loader.dll")
    Copy-Item $bin (Join-Path $staging "devd.exe")
    Copy-Item (Join-Path $repo "LICENSE") (Join-Path $staging "LICENSE")
    Copy-Item (Join-Path $repo "README.md") (Join-Path $staging "README.md")

    $launcher = @'
Dim fso, sh, app, home, code
Set fso = CreateObject("Scripting.FileSystemObject")
Set sh = CreateObject("WScript.Shell")
app = fso.GetParentFolderName(WScript.ScriptFullName)
If Not fso.FileExists(app & "\devd.exe") Then
  MsgBox "devd.exe is missing beside this launcher. Unzip the whole folder, not only the window program.", 16, "devd"
  WScript.Quit 1
End If
Rem A portable install sets DEVD_HOME; the default is %LOCALAPPDATA%\devd.
home = sh.ExpandEnvironmentStrings("%DEVD_HOME%")
If home = "%DEVD_HOME%" Then home = sh.ExpandEnvironmentStrings("%LOCALAPPDATA%") & "\devd"
If Not fso.FileExists(home & "\services\nginx\nginx.exe") Then
  code = sh.Run("""" & app & "\devd.exe"" setup --yes", 1, True)
  If code <> 0 Then
    MsgBox "devd could not finish setting up." & vbCrLf & vbCrLf & "Open PowerShell and run:" & vbCrLf & "  """ & app & "\devd.exe"" setup", 16, "devd"
    WScript.Quit 1
  End If
End If
Call sh.Run("""" & app & "\devd-desktop.exe""", 0, False)
'@
    Set-Content -Path (Join-Path $staging "launch.vbs") -Value $launcher -Encoding ASCII

    $portable = @"
devd portable (Windows)
=======================

This folder is a complete devd: the command line (devd.exe), the window
(devd-desktop.exe), the WebView2 loader the window needs, and a launcher.

First run: double-click launch.vbs. It sets devd up (checks the machine,
downloads PHP, nginx and MariaDB on first use, starts them and creates a site),
then opens the window. The first run can take a few minutes.

Do not move devd-desktop.exe away from WebView2Loader.dll.

If the window does not open:
  - %LOCALAPPDATA%\devd\desktop.log says why.
  - The window needs the Microsoft Edge WebView2 runtime. It ships with current
    Edge; if it is missing, install it from
    https://developer.microsoft.com/microsoft-edge/webview2/
  - The command line still works without it: devd.exe ui opens the same
    interface in your browser.

The installer (devd-setup-$version.exe) does all of this for you, adds devd to
PATH and the Start menu, and is the recommended download when it is published.
"@
    Set-Content -Path (Join-Path $staging "README-PORTABLE.txt") -Value $portable -Encoding UTF8

    Compress-Archive -Path (Join-Path $staging "*") -DestinationPath "dist/$zipName" -Force
}
finally {
    Remove-Item $staging -Recurse -Force -ErrorAction SilentlyContinue
}

$sha = (Get-FileHash "dist/$name" -Algorithm SHA256).Hash.ToLower()
$desktopSha = (Get-FileHash "dist/$desktopName" -Algorithm SHA256).Hash.ToLower()
$zipSha = (Get-FileHash "dist/$zipName" -Algorithm SHA256).Hash.ToLower()
@(
    "$sha  $name"
    "$desktopSha  $desktopName"
    "$zipSha  $zipName"
) | Out-File -Encoding ascii "dist/SHA256SUMS.txt"

# The README tells people to download the installer. A release that does not
# build one sends them looking for a file that is not there, which is exactly
# how a working app looks broken. Build it after the checksum file exists, so
# the installer adds its own line to the same list.
$nsis = Get-Command makensis -ErrorAction SilentlyContinue
if ($nsis) {
    Write-Host "==> installer" -ForegroundColor Cyan
    & (Join-Path $PSScriptRoot "release-installer.ps1") -Nsis $nsis.Source
    if ($LASTEXITCODE -ne 0) { throw "the installer could not be built" }
} else {
    Write-Host "NSIS (makensis) was not found: devd-setup-$version.exe was NOT built." -ForegroundColor Yellow
    Write-Host "Install NSIS 3 or run scripts\\release-installer.ps1 with -Nsis <path> before publishing." -ForegroundColor Yellow
}

Write-Host ""
Write-Host "wrote dist/$name" -ForegroundColor Green
Write-Host "wrote dist/$desktopName" -ForegroundColor Green
Write-Host "wrote dist/$zipName" -ForegroundColor Green
Get-Content "dist/SHA256SUMS.txt"

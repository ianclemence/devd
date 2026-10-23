# Frees a TCP port that Windows has reserved for itself, so a service devd runs
# can bind it again.
#
# Hyper-V and the Windows NAT driver (WinNAT) reserve blocks of ports at boot and
# hand them out for virtual-machine and container traffic. A reservation can land
# on a port a local stack already uses; MariaDB's 3306 is the usual victim. The
# server then fails with
#
#   [ERROR] Can't start server: Bind on TCP/IP port. Got error: 10013
#
# even though nothing is listening and `devd doctor` reports the port as free,
# because the reservation is not a listener. `netsh` shows the reason:
#
#   netsh int ipv4 show excludedportrange protocol=tcp
#
# Stopping and starting WinNAT releases the dynamic reservations for the rest of
# this boot. That needs an administrator, so this script asks for one.
#
#   .\scripts\fix-port-reservation.ps1             # 3306, the MariaDB default
#   .\scripts\fix-port-reservation.ps1 -Port 80    # any other port
#
# It is a workaround, not a fix: the reservations come back after a reboot, and
# Windows may choose different ranges. Run it again if a service stops binding
# after a restart.

param(
    [ValidateRange(1, 65535)]
    [int]$Port = 3306
)

function Get-ReservedRange {
    netsh int ipv4 show excludedportrange protocol=tcp | ForEach-Object {
        if ($_ -match '^\s*(\d+)\s+(\d+)') {
            [pscustomobject]@{ Start = [int]$Matches[1]; End = [int]$Matches[2] }
        }
    }
}

function Test-Reserved([int]$Candidate) {
    foreach ($range in Get-ReservedRange) {
        if ($Candidate -ge $range.Start -and $Candidate -le $range.End) { return $true }
    }
    $false
}

$principal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    Write-Host "port $Port: asking for administrator rights"
    $line = "-NoProfile -ExecutionPolicy Bypass -File `"$PSCommandPath`" -Port $Port"
    $elevated = Start-Process powershell -Verb RunAs -ArgumentList $line -Wait -PassThru
    exit $elevated.ExitCode
}

if (-not (Test-Reserved $Port)) {
    Write-Host "port $Port is not reserved by Windows; nothing to do"
    exit 0
}

Write-Host "port $Port is reserved; stopping and restarting the Windows NAT driver"
$wasRunning = (Get-Service winnat -ErrorAction SilentlyContinue).Status -eq 'Running'
if ($wasRunning) {
    net stop winnat | Out-Null
    Start-Sleep -Seconds 2
}
net start winnat | Out-Null
Start-Sleep -Seconds 2

if (Test-Reserved $Port) {
    Write-Host "port $Port is still reserved; reboot, then run this script again"
    exit 1
}

Write-Host "port $Port is free; start the service again (for example: devd service start mariadb)"
exit 0

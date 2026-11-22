#Requires -Version 5.1
<#
.SYNOPSIS
  Start edgeblend at logon (restarted if it crashes) via a per-user Scheduled Task.

.DESCRIPTION
  Default mode is a DRY RUN: prints what would be registered / removed.
  -Apply registers the task "edgeblend" and removes an edgeblend.lnk in the
  Startup folder if present. -Uninstall removes the task. No admin rights needed.

  The task runs `edgeblend.exe --watchdog --start` as the current user, interactive
  (desktop duplication needs the user's desktop, so this must not be a service).

.PARAMETER ExePath
  Path to edgeblend.exe (default %LOCALAPPDATA%\Programs\edgeblend\edgeblend.exe).

.PARAMETER Apply
  Register the task and remove an edgeblend.lnk from the Startup folder if present.

.PARAMETER Uninstall
  Remove the task.

.EXAMPLE
  .\install-autostart.ps1            # dry run
  .\install-autostart.ps1 -Apply
  .\install-autostart.ps1 -Uninstall -Apply
#>
[CmdletBinding()]
param(
    [string]$ExePath = (Join-Path $env:LOCALAPPDATA 'Programs\edgeblend\edgeblend.exe'),
    [switch]$Apply,
    [switch]$Uninstall
)

$ErrorActionPreference = 'Stop'
$TaskName = 'edgeblend'
$User = "$env:USERDOMAIN\$env:USERNAME"
$StartupShortcut = Join-Path ([Environment]::GetFolderPath('Startup')) 'edgeblend.lnk'

if (-not $Apply) { Write-Host 'DRY RUN (nothing is changed). Re-run with -Apply to do it.' -ForegroundColor Yellow }

if ($Uninstall) {
    if (Get-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue) {
        Write-Host "Would remove scheduled task '$TaskName'"
        if ($Apply) { Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false; Write-Host 'Removed.' }
    } else {
        Write-Host "No scheduled task '$TaskName' found."
    }
    return
}

if ($Apply -and -not (Test-Path $ExePath)) { throw "Not found: $ExePath (use -ExePath)" }

Write-Host "Task      : $TaskName (user $User, interactive, limited rights)"
Write-Host "Action    : $ExePath --watchdog --start"
Write-Host 'Trigger   : at logon of this user, 15 s delay'
Write-Host 'Settings  : no time limit, runs on battery, no idle stop, restart 3x every 1 min, ignore new instance'
if (Test-Path $StartupShortcut) { Write-Host "Would remove Startup shortcut: $StartupShortcut" }
if (-not $Apply) { return }

$action = New-ScheduledTaskAction -Execute $ExePath -Argument '--watchdog --start' -WorkingDirectory (Split-Path $ExePath)
$trigger = New-ScheduledTaskTrigger -AtLogOn -User $User
$trigger.Delay = 'PT15S'
$principal = New-ScheduledTaskPrincipal -UserId $User -LogonType Interactive -RunLevel Limited
$settings = New-ScheduledTaskSettingsSet `
    -ExecutionTimeLimit ([TimeSpan]::Zero) `
    -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -DontStopOnIdleEnd `
    -RestartCount 3 -RestartInterval (New-TimeSpan -Minutes 1) `
    -MultipleInstances IgnoreNew -StartWhenAvailable

Register-ScheduledTask -TaskName $TaskName -Action $action -Trigger $trigger `
    -Principal $principal -Settings $settings -Force | Out-Null
Write-Host "Registered scheduled task '$TaskName'." -ForegroundColor Green

if (Test-Path $StartupShortcut) {
    Remove-Item $StartupShortcut
    Write-Host "Removed Startup shortcut: $StartupShortcut"
}

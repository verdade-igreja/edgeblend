#Requires -Version 5.1
<#
.SYNOPSIS
  Prepare / install VirtualDrivers Virtual Display Driver for a 5760x1080 edgeblend canvas.

.DESCRIPTION
  Default mode is a DRY RUN: prints download URLs, config path, XML contents, and install
  commands without changing the system.

  Real work requires -Apply (or -Uninstall) and an elevated PowerShell session.

  Based on VirtualDrivers/Virtual-Display-Driver community silent-install.ps1
  (nefcon + Root\MttVDD). See NOTES.md in this folder.

.PARAMETER Width
  Virtual monitor width (default 5760, an example value).

.PARAMETER Height
  Virtual monitor height (default 1080, an example value).

.PARAMETER Refresh
  Primary refresh rate in Hz (default 60). A 30 Hz fallback mode is always included
  when Refresh is not 30.

.PARAMETER GpuFriendlyName
  Value for vdd_settings.xml <gpu><friendlyname>. Use the DXGI adapter description
  of your GPU, or "default" (the default) to let the driver auto-pick.

.PARAMETER Apply
  Perform download, write config, certificate trust, and driver install/update.

.PARAMETER Uninstall
  Remove the Root\MttVDD device via nefcon (admin). Does not delete C:\VirtualDisplayDriver
  unless -RemoveConfig is also passed.

.PARAMETER RemoveConfig
  With -Uninstall, also remove C:\VirtualDisplayDriver if present.

.EXAMPLE
  .\setup.ps1
  # Dry run only.

.EXAMPLE
  .\setup.ps1 -Apply
  # Elevated: write 5760x1080 config and install/update the driver.
#>
[CmdletBinding(DefaultParameterSetName = 'Install')]
param(
    [Parameter(ParameterSetName = 'Install')]
    [Parameter(ParameterSetName = 'Uninstall')]
    [ValidateRange(640, 7680)]
    [int] $Width = 5760,

    [Parameter(ParameterSetName = 'Install')]
    [Parameter(ParameterSetName = 'Uninstall')]
    [ValidateRange(480, 4320)]
    [int] $Height = 1080,

    [Parameter(ParameterSetName = 'Install')]
    [Parameter(ParameterSetName = 'Uninstall')]
    [ValidateRange(24, 500)]
    [int] $Refresh = 60,

    [Parameter(ParameterSetName = 'Install')]
    [string] $GpuFriendlyName = 'default',

    [Parameter(ParameterSetName = 'Install')]
    [switch] $Apply,

    [Parameter(Mandatory = $true, ParameterSetName = 'Uninstall')]
    [switch] $Uninstall,

    [Parameter(ParameterSetName = 'Uninstall')]
    [switch] $RemoveConfig
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# --- Constants (pinned versions; bump after reading NOTES.md / GitHub releases) ---
$DriverTag = '25.7.23'
$DriverZipUrl = "https://github.com/VirtualDrivers/Virtual-Display-Driver/releases/download/$DriverTag/VirtualDisplayDriver-x86.Driver.Only.zip"
$NefConZipUrl = 'https://github.com/nefarius/nefcon/releases/download/v1.21.1/nefcon_v1.21.1.zip'
$HardwareId = 'Root\MttVDD'
$ConfigDir = Join-Path -Path $env:SystemDrive -ChildPath 'VirtualDisplayDriver'
$ConfigPath = Join-Path -Path $ConfigDir -ChildPath 'vdd_settings.xml'
$ScriptDir = $PSScriptRoot
if ([string]::IsNullOrEmpty($ScriptDir)) {
    $ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
}

function Test-IsAdministrator {
    $id = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = New-Object Security.Principal.WindowsPrincipal($id)
    return $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

function Assert-Administrator {
    param([string] $Action)
    if (-not (Test-IsAdministrator)) {
        Write-Host @"
Refusing to ${Action}: this PowerShell session is not elevated.

Right-click PowerShell -> Run as administrator, then re-run with -${Action}.
"@ -ForegroundColor Red
        exit 1
    }
}

function Get-VddSettingsXml {
    param(
        [int] $Width,
        [int] $Height,
        [int] $Refresh,
        [string] $GpuFriendlyName
    )

    $rates = New-Object System.Collections.Generic.List[int]
    [void]$rates.Add($Refresh)
    if ($Refresh -ne 30) {
        [void]$rates.Add(30)
    }

    $globalRates = ($rates | ForEach-Object { "        <g_refresh_rate>$_</g_refresh_rate>" }) -join "`r`n"
    $resBlocks = foreach ($hz in $rates) {
        @"
        <resolution>
            <width>$Width</width>
            <height>$Height</height>
            <refresh_rate>$hz</refresh_rate>
        </resolution>
"@
    }
    $resolutions = ($resBlocks -join "`r`n").TrimEnd()

    return @"
<?xml version='1.0' encoding='utf-8'?>
<!-- edgeblend virtual canvas - written by tools/virtual-display/setup.ps1 -->
<vdd_settings>
    <monitors>
        <count>1</count>
    </monitors>
    <gpu>
        <friendlyname>$GpuFriendlyName</friendlyname>
    </gpu>
    <global>
$globalRates
    </global>
    <resolutions>
$resolutions
    </resolutions>
    <logging>
        <SendLogsThroughPipe>true</SendLogsThroughPipe>
        <logging>false</logging>
        <debuglogging>false</debuglogging>
    </logging>
    <colour>
        <SDR10bit>false</SDR10bit>
        <HDRPlus>false</HDRPlus>
        <ColourFormat>RGB</ColourFormat>
    </colour>
    <cursor>
        <HardwareCursor>true</HardwareCursor>
        <CursorMaxX>128</CursorMaxX>
        <CursorMaxY>128</CursorMaxY>
        <AlphaCursorSupport>true</AlphaCursorSupport>
        <XorCursorSupportLevel>2</XorCursorSupportLevel>
    </cursor>
    <edid>
        <CustomEdid>false</CustomEdid>
        <PreventSpoof>false</PreventSpoof>
        <EdidCeaOverride>false</EdidCeaOverride>
    </edid>
    <edid_integration>
        <enabled>false</enabled>
        <auto_configure_from_edid>false</auto_configure_from_edid>
        <edid_profile_path>EDID/monitor_profile.xml</edid_profile_path>
        <override_manual_settings>false</override_manual_settings>
        <fallback_on_error>true</fallback_on_error>
    </edid_integration>
    <auto_resolutions>
        <enabled>false</enabled>
        <source_priority>manual</source_priority>
        <preferred_mode>
            <use_edid_preferred>false</use_edid_preferred>
            <fallback_width>$Width</fallback_width>
            <fallback_height>$Height</fallback_height>
            <fallback_refresh>$Refresh</fallback_refresh>
        </preferred_mode>
    </auto_resolutions>
</vdd_settings>
"@
}

function Find-NefConExe {
    param([string] $Root)
    $candidates = @(
        (Join-Path -Path $Root -ChildPath 'x64\nefconw.exe'),
        (Join-Path -Path $Root -ChildPath 'nefconw.exe')
    )
    foreach ($c in $candidates) {
        if (Test-Path -LiteralPath $c) {
            return $c
        }
    }
    $found = Get-ChildItem -LiteralPath $Root -Filter 'nefconw.exe' -Recurse -ErrorAction SilentlyContinue |
        Select-Object -First 1
    if ($null -ne $found) {
        return $found.FullName
    }
    throw "nefconw.exe not found under $Root"
}

function Find-DriverInf {
    param([string] $Root)
    $direct = Join-Path -Path $Root -ChildPath 'VirtualDisplayDriver\MttVDD.inf'
    if (Test-Path -LiteralPath $direct) {
        return $direct
    }
    $found = Get-ChildItem -LiteralPath $Root -Filter 'MttVDD.inf' -Recurse -ErrorAction SilentlyContinue |
        Select-Object -First 1
    if ($null -ne $found) {
        return $found.FullName
    }
    throw "MttVDD.inf not found under $Root"
}

function Find-DriverCat {
    param([string] $Root)
    $found = Get-ChildItem -LiteralPath $Root -Filter 'mttvdd.cat' -Recurse -ErrorAction SilentlyContinue |
        Select-Object -First 1
    if ($null -eq $found) {
        $found = Get-ChildItem -LiteralPath $Root -Filter '*.cat' -Recurse -ErrorAction SilentlyContinue |
            Select-Object -First 1
    }
    if ($null -eq $found) {
        throw "Driver .cat certificate package not found under $Root"
    }
    return $found.FullName
}

function Test-VddDevicePresent {
    try {
        $dev = Get-PnpDevice -ErrorAction SilentlyContinue |
            Where-Object { $_.InstanceId -like '*MttVDD*' -or $_.HardwareID -contains $HardwareId }
        return ($null -ne $dev)
    }
    catch {
        return $false
    }
}

function Write-ConfigFile {
    param(
        [string] $Path,
        [string] $Xml,
        [switch] $WhatIfOnly
    )
    if ($WhatIfOnly) {
        Write-Host "[dry-run] Would ensure directory: $(Split-Path -Parent $Path)"
        Write-Host "[dry-run] Would write: $Path"
        return
    }
    $dir = Split-Path -Parent $Path
    if (-not (Test-Path -LiteralPath $dir)) {
        New-Item -ItemType Directory -Path $dir -Force | Out-Null
    }
    if (Test-Path -LiteralPath $Path) {
        $existing = Get-Content -LiteralPath $Path -Raw -ErrorAction SilentlyContinue
        if ($existing -eq $Xml) {
            Write-Host "Config already up to date: $Path"
            return
        }
        $stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
        $backup = "$Path.bak-$stamp"
        Copy-Item -LiteralPath $Path -Destination $backup -Force
        Write-Host "Backed up previous config to $backup"
    }
    # UTF-8 without BOM (PS 5.1 Set-Content -Encoding UTF8 writes a BOM)
    $utf8NoBom = New-Object System.Text.UTF8Encoding $false
    [System.IO.File]::WriteAllText($Path, $Xml, $utf8NoBom)
    Write-Host "Wrote $Path"
}

function Install-DriverCertificates {
    param([string] $CatPath)
    Write-Host "Trusting publisher cert(s) from $(Split-Path -Leaf $CatPath) ..."
    $catBytes = [System.IO.File]::ReadAllBytes($CatPath)
    $certificates = New-Object System.Security.Cryptography.X509Certificates.X509Certificate2Collection
    $certificates.Import($catBytes)
    $exportDir = Join-Path -Path $env:TEMP -ChildPath ('VDDCerts-' + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $exportDir -Force | Out-Null
    try {
        foreach ($cert in $certificates) {
            $cerPath = Join-Path -Path $exportDir -ChildPath ($cert.Thumbprint + '.cer')
            [System.IO.File]::WriteAllBytes(
                $cerPath,
                $cert.Export([System.Security.Cryptography.X509Certificates.X509ContentType]::Cert)
            )
            Import-Certificate -FilePath $cerPath -CertStoreLocation 'Cert:\LocalMachine\TrustedPublisher' | Out-Null
            Write-Host "  Imported thumbprint $($cert.Thumbprint)"
        }
    }
    finally {
        Remove-Item -LiteralPath $exportDir -Recurse -Force -ErrorAction SilentlyContinue
    }
}

function Invoke-NefConInstall {
    param(
        [string] $NefConExe,
        [string] $InfPath
    )
    $infDir = Split-Path -Parent $InfPath
    $infLeaf = Split-Path -Leaf $InfPath
    Write-Host "Installing device $HardwareId from $InfPath ..."
    Push-Location -LiteralPath $infDir
    try {
        # nefcon "install" is the silent-install.ps1 pattern (create ROOT node + bind INF)
        # nefconw.exe is a GUI-subsystem exe: '&' would not wait or set $LASTEXITCODE.
        $p = Start-Process -FilePath $NefConExe -ArgumentList @('install', ".\$infLeaf", $HardwareId) `
            -WorkingDirectory $infDir -Wait -PassThru -NoNewWindow
        $code = $p.ExitCode
        if ($null -ne $code -and $code -ne 0 -and $code -ne 3010) {
            throw "nefcon install failed with exit code $code"
        }
        if ($code -eq 3010) {
            Write-Warning "Driver install requested a reboot (exit 3010)."
        }
    }
    finally {
        Pop-Location
    }
}

function Invoke-NefConRemove {
    param([string] $NefConExe)
    Write-Host "Removing device $HardwareId ..."
    $p = Start-Process -FilePath $NefConExe -ArgumentList @('remove', $HardwareId) -Wait -PassThru -NoNewWindow
    $code = $p.ExitCode
    if ($null -ne $code -and $code -ne 0 -and $code -ne 3010) {
        throw "nefcon remove failed with exit code $code"
    }
}

function Show-DryRun {
    param(
        [string] $Xml,
        [switch] $ForUninstall
    )

    Write-Host ''
    Write-Host '=== DRY RUN (no changes) ===' -ForegroundColor Cyan
    Write-Host "Config directory : $ConfigDir"
    Write-Host "Config file      : $ConfigPath"
    Write-Host "Driver zip URL   : $DriverZipUrl"
    Write-Host "NefCon zip URL   : $NefConZipUrl"
    Write-Host "Hardware ID      : $HardwareId"
    Write-Host "GPU friendlyname : $GpuFriendlyName"
    Write-Host "Mode             : ${Width}x${Height}@$Refresh (plus 30 Hz fallback if needed)"
    Write-Host ''

    if ($ForUninstall) {
        Write-Host 'Would run (elevated):'
        Write-Host "  nefconw.exe remove `"$HardwareId`""
        if ($RemoveConfig) {
            Write-Host "  Remove-Item -LiteralPath `"$ConfigDir`" -Recurse -Force"
        }
        Write-Host ''
        Write-Host 'Re-run with -Uninstall in an elevated prompt to apply.'
        return
    }

    Write-Host '--- vdd_settings.xml contents ---'
    Write-Host $Xml
    Write-Host '--- end XML ---'
    Write-Host ''
    Write-Host 'Would (elevated, -Apply):'
    Write-Host "  1. Download driver zip + nefcon into `$env:TEMP\edgeblend-vdd-setup\*"
    Write-Host "  2. Import SignPath publisher cert(s) from mttvdd.cat into LocalMachine\TrustedPublisher"
    Write-Host "  3. Write config to $ConfigPath (backup previous if different)"
    Write-Host "  4. If device missing: nefconw.exe install .\MttVDD.inf `"$HardwareId`""
    Write-Host '     If already present: leave device; config-only update (disable/enable PnP to reload)'
    Write-Host "  5. Copy a mirror of the XML next to this script for reference"
    Write-Host ''
    Write-Host 'Re-run with -Apply in an elevated PowerShell to perform these steps.'
}

# --- Main ---
$xml = Get-VddSettingsXml -Width $Width -Height $Height -Refresh $Refresh -GpuFriendlyName $GpuFriendlyName
$doWork = $Apply -or $Uninstall

if (-not $doWork) {
    Show-DryRun -Xml $xml -ForUninstall:$Uninstall
    exit 0
}

if ($Uninstall) {
    Assert-Administrator -Action 'Uninstall'
    $workRoot = Join-Path -Path $env:TEMP -ChildPath ('edgeblend-vdd-uninstall-' + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $workRoot -Force | Out-Null
    try {
        $nefZip = Join-Path -Path $workRoot -ChildPath 'nefcon.zip'
        Write-Host "Downloading nefcon ..."
        Invoke-WebRequest -Uri $NefConZipUrl -OutFile $nefZip -UseBasicParsing
        Expand-Archive -LiteralPath $nefZip -DestinationPath $workRoot -Force
        $nef = Find-NefConExe -Root $workRoot
        if (Test-VddDevicePresent) {
            Invoke-NefConRemove -NefConExe $nef
        }
        else {
            Write-Host "No $HardwareId device found; nothing to remove."
        }
        if ($RemoveConfig -and (Test-Path -LiteralPath $ConfigDir)) {
            Remove-Item -LiteralPath $ConfigDir -Recurse -Force
            Write-Host "Removed $ConfigDir"
        }
        Write-Host 'Uninstall finished.'
    }
    finally {
        Remove-Item -LiteralPath $workRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
    exit 0
}

# -Apply path
Assert-Administrator -Action 'Apply'

$workRoot = Join-Path -Path $env:TEMP -ChildPath ('edgeblend-vdd-setup-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $workRoot -Force | Out-Null

try {
    Write-ConfigFile -Path $ConfigPath -Xml $xml

    $mirror = Join-Path -Path $ScriptDir -ChildPath 'vdd_settings.edgeblend.xml'
    Write-ConfigFile -Path $mirror -Xml $xml

    $already = Test-VddDevicePresent
    if ($already) {
        Write-Host "Device $HardwareId already present — updating config only (idempotent)."
        Write-Host 'Reload: Device Manager -> Display adapters -> Virtual Display Driver -> Disable device, then Enable,'
        Write-Host 'or use Virtual Driver Control "Reload driver", then set 5760x1080 in Settings > Display.'
        exit 0
    }

    Write-Host "Downloading driver $DriverTag ..."
    $driverZip = Join-Path -Path $workRoot -ChildPath 'driver.zip'
    Invoke-WebRequest -Uri $DriverZipUrl -OutFile $driverZip -UseBasicParsing
    Expand-Archive -LiteralPath $driverZip -DestinationPath $workRoot -Force

    Write-Host 'Downloading nefcon ...'
    $nefZip = Join-Path -Path $workRoot -ChildPath 'nefcon.zip'
    Invoke-WebRequest -Uri $NefConZipUrl -OutFile $nefZip -UseBasicParsing
    Expand-Archive -LiteralPath $nefZip -DestinationPath $workRoot -Force

    $nef = Find-NefConExe -Root $workRoot
    $inf = Find-DriverInf -Root $workRoot
    $cat = Find-DriverCat -Root $workRoot

    Install-DriverCertificates -CatPath $cat
    Invoke-NefConInstall -NefConExe $nef -InfPath $inf

    Start-Sleep -Seconds 3
    if (Test-VddDevicePresent) {
        Write-Host "Install OK — $HardwareId is present."
    }
    else {
        Write-Warning "Install finished but PnP device was not detected yet. Check Device Manager / reboot."
    }
    Write-Host "Next: see AFTER-INSTALL.md (set ${Width}x${Height}, arrange displays, edgeblend --list)."
}
finally {
    Remove-Item -LiteralPath $workRoot -Recurse -Force -ErrorAction SilentlyContinue
}

; edgeblend installer (Inno Setup 6). Built by CI: ISCC /DAppVersion=1.2.3 installer\edgeblend.iss
; Paths are relative to this file's folder.

#ifndef AppVersion
  #define AppVersion "0.0.0-dev"
#endif
#ifndef OutDir
  #define OutDir "..\dist"
#endif

[Setup]
AppId={{57046337-FF34-4909-99E3-AA45E3E7715E}
AppName=edgeblend
AppVersion={#AppVersion}
AppPublisher=edgeblend contributors
AppPublisherURL=https://github.com/verdade-igreja/edgeblend
AppSupportURL=https://github.com/verdade-igreja/edgeblend
LicenseFile=..\LICENSE
; Per-user install, no admin. With lowest privileges {autopf} = %LOCALAPPDATA%\Programs.
PrivilegesRequired=lowest
DefaultDirName={autopf}\edgeblend
DisableProgramGroupPage=yes
DisableDirPage=auto
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir={#OutDir}
OutputBaseFilename=edgeblend-setup-{#AppVersion}
UninstallDisplayIcon={app}\edgeblend.exe
Compression=lzma2
SolidCompression=yes
WizardStyle=modern

[Tasks]
Name: "autostart"; Description: "Start edgeblend automatically when I log in"
Name: "desktopicon"; Description: "Create a &desktop icon"; Flags: unchecked

[Files]
Source: "..\target\release\edgeblend.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\tools\autostart\install-autostart.ps1"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\blend.example.toml"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\edgeblend"; Filename: "{app}\edgeblend.exe"
Name: "{autodesktop}\edgeblend"; Filename: "{app}\edgeblend.exe"; Tasks: desktopicon

[Run]
; Registers the logon Scheduled Task (also removes an edgeblend.lnk in the Startup folder, if present).
Filename: "powershell.exe"; \
  Parameters: "-NoProfile -ExecutionPolicy Bypass -File ""{app}\install-autostart.ps1"" -ExePath ""{app}\edgeblend.exe"" -Apply"; \
  Flags: runhidden waituntilterminated; Tasks: autostart
Filename: "{app}\edgeblend.exe"; Parameters: "--watchdog --start"; \
  Description: "Launch edgeblend"; Flags: nowait postinstall skipifsilent

[UninstallRun]
Filename: "powershell.exe"; \
  Parameters: "-NoProfile -ExecutionPolicy Bypass -File ""{app}\install-autostart.ps1"" -Uninstall -Apply"; \
  Flags: runhidden waituntilterminated; RunOnceId: "RemoveAutostartTask"

[Code]
// Kill every edgeblend.exe (the watchdog parent and its child share the name). Killing is
// needed because the watchdog would otherwise restart the app and keep the exe locked.
procedure StopEdgeblend;
var
  ResultCode: Integer;
begin
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM edgeblend.exe', '', SW_HIDE,
       ewWaitUntilTerminated, ResultCode);
  Sleep(1000);
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  StopEdgeblend;
  Result := '';
end;

function InitializeUninstall(): Boolean;
begin
  StopEdgeblend;
  Result := True;
end;

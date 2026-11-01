; Windows installer (Inno Setup 6). Per-user install, no admin prompt.
; iscc /DAppVersion=0.1.0 /DExe=target\release\monitorctl.exe /DIcon=<path to monitorctl.ico> installer\monitorctl.iss
; The .ico is written by build.rs into target\release\build\monitorctl-*\out\.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif

[Setup]
AppId=monitorctl
AppName=monitorctl
AppVersion={#AppVersion}
AppPublisher=edsuuu
AppPublisherURL=https://github.com/edsuuu/monitorctl
DefaultDirName={localappdata}\Programs\monitorctl
DisableProgramGroupPage=yes
DisableDirPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputBaseFilename=monitorctl-setup-x64
SetupIconFile={#Icon}
UninstallDisplayIcon={app}\monitorctl.exe
WizardStyle=modern
CloseApplications=yes
SourceDir=..

[Languages]
Name: "brazilianportuguese"; MessagesFile: "compiler:Languages\BrazilianPortuguese.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"

[Files]
Source: "{#Exe}"; DestDir: "{app}"; DestName: "monitorctl.exe"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\monitorctl"; Filename: "{app}\monitorctl.exe"
Name: "{autodesktop}\monitorctl"; Filename: "{app}\monitorctl.exe"; Tasks: desktopicon

[Registry]
; "Iniciar com o sistema" is written by the app itself; only clean it up on uninstall.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueName: "monitorctl"; ValueType: none; Flags: uninsdeletevalue dontcreatekey

[Run]
Filename: "{app}\monitorctl.exe"; Description: "{cm:LaunchProgram,monitorctl}"; Flags: nowait postinstall skipifsilent

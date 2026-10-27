; Inno Setup script for Audio Editor.
; Build with scripts/build-installer.ps1, which passes the version:
;   ISCC installer\audio_editor.iss /DAppVersion=0.1.0

#define AppName "Audio Editor"
#define AppExe "audio_editor.exe"
; Root is passed by scripts/build-installer.ps1 as /DRoot=<repo>\; the
; fallback ("..\") works when ISCC is run from the repo root.
#ifndef Root
#define Root "..\"
#endif

[Setup]
; Fixed GUID, never change: upgrades in place over older installs.
AppId={{33A79C24-EFF1-4C51-9F84-B7C35CD05F71}
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher={#AppName}
DefaultDirName={autopf}\{#AppName}
DisableProgramGroupPage=yes
; Per-user by default, but the user can choose elevated (all-users) in the dialog.
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
MinVersion=10.0
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
SetupIconFile={#Root}assets\icon\app.ico
UninstallDisplayIcon={app}\{#AppExe}
; Close a running copy before updating the exe.
CloseApplications=yes
OutputDir={#Root}target\installer
OutputBaseFilename=AudioEditor-Setup-{#AppVersion}
Compression=lzma
SolidCompression=yes
WizardStyle=modern

[Files]
Source: "{#Root}target\release\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Root}README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Root}THIRD-PARTY-NOTICES.txt"; DestDir: "{app}"; Flags: ignoreversion

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; \
    GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#AppExe}"; Description: "Launch {#AppName}"; \
    Flags: nowait postinstall skipifsilent

[Uninstall]
; Default behavior only: removes the files installed above. %APPDATA% and
; project folders are deliberately left untouched.

; Inno Setup script for CutMark Audio Editor.
; Build with scripts/build-installer.ps1, which passes the version:
;   ISCC installer\cutmark.iss /DAppVersion=1.2.0 "/DBinDir=<workspace>\target\release\"

#define AppName "CutMark Audio Editor"
#define AppExe "cutmark-audio.exe"
; Root is passed by scripts/build-installer.ps1 as /DRoot=<repo>\; the
; fallback ("..\") works when ISCC is run from the repo root.
#ifndef Root
#define Root "..\"
#endif
#ifndef BinDir
#define BinDir "..\..\..\target\release\"
#endif
; The setup program's icon is the exe's, from the shared brand assets.
#ifndef BrandDir
#define BrandDir Root + "..\..\crates\ui-kit\assets\brand\"
#endif
; The font licenses live beside the fonts in the shared ui-kit crate.
#ifndef FontsDir
#define FontsDir Root + "..\..\crates\ui-kit\assets\fonts\"
#endif
#ifndef FFmpegDir
#define FFmpegDir Root + "..\..\third_party\ffmpeg\"
#endif

[Setup]
; Fixed GUID, never change: upgrades in place over older installs.
AppId={{33A79C24-EFF1-4C51-9F84-B7C35CD05F71}
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher=JD
DefaultDirName={autopf}\CutMark
DisableProgramGroupPage=yes
; Per-user by default, but the user can choose elevated (all-users) in the dialog.
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
MinVersion=10.0
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
SetupIconFile={#BrandDir}icon-audio.ico
UninstallDisplayIcon={app}\{#AppExe}
; Close a running copy before updating the exe.
CloseApplications=yes
OutputDir={#Root}dist
OutputBaseFilename=CutMark-Setup-{#AppVersion}
Compression=lzma
SolidCompression=yes
WizardStyle=modern

[Files]
Source: "{#BinDir}{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#FFmpegDir}dist\bin\*.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#FFmpegDir}BUILDINFO.txt"; DestDir: "{app}\licenses"; DestName: "FFmpeg-BUILDINFO.txt"; Flags: ignoreversion
Source: "{#FFmpegDir}dist\LICENSE.txt"; DestDir: "{app}\licenses"; DestName: "FFmpeg-LICENSE.txt"; Flags: ignoreversion
Source: "{#Root}licenses\*"; DestDir: "{app}\licenses"; Flags: ignoreversion recursesubdirs
Source: "{#FontsDir}OFL-InstrumentSans.txt"; DestDir: "{app}\licenses"; Flags: ignoreversion
Source: "{#FontsDir}OFL-MartianMono.txt"; DestDir: "{app}\licenses"; Flags: ignoreversion
Source: "{#Root}README.md"; DestDir: "{app}\licenses"; DestName: "README.md"; Flags: ignoreversion

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

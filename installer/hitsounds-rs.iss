; Inno Setup script for the hitsounds-rs installer (D-85). Built by tools/package.ps1,
; which passes the version and the folder with the release build:
;
;     iscc /DAppVersion=0.1.0-alpha.1 /DNumericVersion=0.1.0 /DSourceDir=<dir> /DOutputDir=<dir> installer\hitsounds-rs.iss
;
; The install is per user (no UAC prompt), so the app's own updater (D-84) can run a
; newer setup silently: /VERYSILENT /SUPPRESSMSGBOXES /CLOSEAPPLICATIONS, plus
; /relaunch=1 to start the app again afterwards.

#ifndef AppVersion
  #define AppVersion "0.0.0-dev"
#endif
#ifndef NumericVersion
  #define NumericVersion "0.0.0"
#endif
#ifndef SourceDir
  #define SourceDir "..\target\release"
#endif
#ifndef OutputDir
  #define OutputDir "..\dist"
#endif

[Setup]
; Never change the AppId: it is how a new setup finds the existing install.
AppId={{C3F15F39-5E7B-45E3-BD2C-B5CBD72F844F}
AppName=hitsounds-rs
AppVersion={#AppVersion}
AppVerName=hitsounds-rs {#AppVersion}
AppPublisher=hitsounds-rs
VersionInfoVersion={#NumericVersion}
VersionInfoProductTextVersion={#AppVersion}
DefaultDirName={localappdata}\Programs\hitsounds-rs
DefaultGroupName=hitsounds-rs
DisableProgramGroupPage=yes
DisableDirPage=auto
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputDir={#OutputDir}
OutputBaseFilename=hitsounds-rs-{#AppVersion}-setup
SetupIconFile=..\crates\hitsounds-rs\assets\icon.ico
UninstallDisplayIcon={app}\hitsounds-rs.exe
UninstallDisplayName=hitsounds-rs
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
; The updater starts setup while the app is exiting; let Restart Manager finish closing it.
CloseApplications=force
RestartApplications=no

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SourceDir}\hitsounds-rs.exe"; DestDir: "{app}"; Flags: ignoreversion
; Line tables for crash.log backtraces (D-75). Rust names it with an underscore, and the exe looks for that name.
Source: "{#SourceDir}\hitsounds_rs.pdb"; DestDir: "{app}"; Flags: ignoreversion
; ONNX Runtime for drum separation (D-74). The model is never shipped (D-71).
Source: "{#SourceDir}\onnxruntime.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\onnxruntime-LICENSE.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\onnxruntime-ThirdPartyNotices.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion
Source: "..\THIRD_PARTY_NOTICES.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\hitsounds-rs"; Filename: "{app}\hitsounds-rs.exe"
Name: "{autodesktop}\hitsounds-rs"; Filename: "{app}\hitsounds-rs.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\hitsounds-rs.exe"; Description: "{cm:LaunchProgram,hitsounds-rs}"; Flags: nowait postinstall skipifsilent
; "Restart to update" (D-84): the silent update starts the app again.
Filename: "{app}\hitsounds-rs.exe"; Flags: nowait; Check: Relaunch

[UninstallDelete]
; Leftovers of a portable-style update, if one ever ran here.
Type: files; Name: "{app}\*.old"

[Code]
function Relaunch: Boolean;
begin
  Result := WizardSilent and (ExpandConstant('{param:relaunch|0}') = '1');
end;

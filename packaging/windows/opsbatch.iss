; Inno Setup 6 script. Build with:  iscc /DVersion=0.1.0 /DArch=x86_64 /DSourceDir=..\..\target\release /DOutputDir=..\..\dist opsbatch.iss
#ifndef Version
  #error Pass /DVersion=x.y.z
#endif
#ifndef Arch
  #define Arch "x86_64"
#endif
#ifndef SourceDir
  #define SourceDir "..\..\target\release"
#endif
#ifndef OutputDir
  #define OutputDir "..\..\dist"
#endif

[Setup]
; The AppId keeps upgrades in place (a newer installer replaces the old install). Do not change it.
AppId={{6F0E8E52-1C65-4C9D-9E4B-0B6C7D1A2E31}
AppName=OpsBatch
AppVersion={#Version}
AppPublisher=Mio
DefaultDirName={autopf}\OpsBatch
DefaultGroupName=OpsBatch
UninstallDisplayIcon={app}\opsbatch.exe
SetupIconFile=..\icons\icon.ico
OutputDir={#OutputDir}
; Name is part of the update protocol (crates/ops-core/src/update.rs::installer_asset_name).
OutputBaseFilename=OpsBatch-{#Version}-windows-{#Arch}-setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
CloseApplications=yes
RestartApplications=no
#if Arch == "aarch64"
ArchitecturesAllowed=arm64
ArchitecturesInstallIn64BitMode=arm64
#else
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
#endif

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "chinesesimplified"; MessagesFile: "ChineseSimplified.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SourceDir}\opsbatch.exe"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\OpsBatch"; Filename: "{app}\opsbatch.exe"
Name: "{autodesktop}\OpsBatch"; Filename: "{app}\opsbatch.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\opsbatch.exe"; Description: "{cm:LaunchProgram,OpsBatch}"; Flags: nowait postinstall skipifsilent

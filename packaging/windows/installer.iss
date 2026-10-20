; Inno Setup script for WLED Ambilight.
; Built by the release workflow:  iscc /DAppVersion=1.2.3 /DSourceDir=<dir> installer.iss

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef SourceDir
  #define SourceDir "..\..\target\release"
#endif

[Setup]
AppId={{6B1C2E7A-3F5D-4C8B-9E21-7A4D5F0C9B13}
AppName=WLED Ambilight
AppVersion={#AppVersion}
AppVerName=WLED Ambilight {#AppVersion}
AppPublisher=Patrick Weppelmann
AppPublisherURL=https://github.com/Smotherer007/wledambilight
AppSupportURL=https://github.com/Smotherer007/wledambilight/issues
AppCopyright=Copyright (C) 2026 Patrick Weppelmann
DefaultDirName={autopf}\WLED Ambilight
DefaultGroupName=WLED Ambilight
DisableProgramGroupPage=yes
LicenseFile=..\..\LICENSE
OutputDir={#SourceDir}
OutputBaseFilename=wled-ambilight-{#AppVersion}-windows-x86_64-setup
SetupIconFile=..\..\assets\icons\app.ico
UninstallDisplayIcon={app}\wled-ambilight.exe
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequiredOverridesAllowed=dialog
MinVersion=10.0.18362

[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl"
Name: "de"; MessagesFile: "compiler:Languages\German.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked
Name: "autostart"; Description: "{cm:AutoStartProgram,WLED Ambilight}"; GroupDescription: "{cm:AutoStartProgramGroupDescription}"; Flags: unchecked

[Files]
Source: "{#SourceDir}\wled-ambilight.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion
Source: "..\..\README.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\WLED Ambilight"; Filename: "{app}\wled-ambilight.exe"
Name: "{autodesktop}\WLED Ambilight"; Filename: "{app}\wled-ambilight.exe"; Tasks: desktopicon
Name: "{userstartup}\WLED Ambilight"; Filename: "{app}\wled-ambilight.exe"; Parameters: "--minimized"; Tasks: autostart

[Run]
Filename: "{app}\wled-ambilight.exe"; Description: "{cm:LaunchProgram,WLED Ambilight}"; Flags: nowait postinstall skipifsilent

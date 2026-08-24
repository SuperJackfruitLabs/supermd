; Inno Setup script for SuperMD. Build:
;   iscc /DAppVersion=<ver> scripts\windows\supermd.iss
#ifndef AppVersion
#define AppVersion "0.0.0"
#endif

[Setup]
AppName=SuperMD
AppVersion={#AppVersion}
AppPublisher=SuperJackfruitLabs
AppPublisherURL=https://supermd.app
DefaultDirName={autopf}\SuperMD
DefaultGroupName=SuperMD
OutputDir=..\..\dist
OutputBaseFilename=SuperMD-Setup-{#AppVersion}
UninstallDisplayIcon={app}\supermd.exe
ChangesAssociations=yes
DisableProgramGroupPage=yes

[Files]
Source: "..\..\target\release\supermd.exe"; DestDir: "{app}"
Source: "..\..\dist\default-plugins\*"; DestDir: "{app}\plugins"; Flags: recursesubdirs createallsubdirs

[Icons]
Name: "{group}\SuperMD"; Filename: "{app}\supermd.exe"
Name: "{autodesktop}\SuperMD"; Filename: "{app}\supermd.exe"; Tasks: desktopicon

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; Flags: unchecked
Name: "mdassoc"; Description: "Associate .md and .markdown files with SuperMD"

[Registry]
Root: HKA; Subkey: "Software\Classes\.md\OpenWithProgids"; ValueType: string; ValueName: "SuperMD.md"; ValueData: ""; Flags: uninsdeletevalue; Tasks: mdassoc
Root: HKA; Subkey: "Software\Classes\.markdown\OpenWithProgids"; ValueType: string; ValueName: "SuperMD.md"; ValueData: ""; Flags: uninsdeletevalue; Tasks: mdassoc
Root: HKA; Subkey: "Software\Classes\SuperMD.md"; ValueType: string; ValueName: ""; ValueData: "Markdown Document"; Flags: uninsdeletekey; Tasks: mdassoc
Root: HKA; Subkey: "Software\Classes\SuperMD.md\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\supermd.exe,0"; Tasks: mdassoc
Root: HKA; Subkey: "Software\Classes\SuperMD.md\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\supermd.exe"" ""%1"""; Tasks: mdassoc

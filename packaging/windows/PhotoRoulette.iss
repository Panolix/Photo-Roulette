#define MyAppName "Photo Roulette"
#ifndef MyAppVersion
  #define MyAppVersion "0.3.0"
#endif
#define MyAppPublisher "Panagiotis Smponias"
#define MyAppURL "https://github.com/Panolix/Photo-Roulette"
#define MyAppExeName "photo-roulette.exe"
#define ProjectRoot "..\.."

[Setup]
AppId={{E595D8A1-252D-44DF-B376-5CD0CB18CA46}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppVerName={#MyAppName} {#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}
AppUpdatesURL={#MyAppURL}/releases
DefaultDirName={localappdata}\Programs\Photo Roulette
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
OutputDir={#ProjectRoot}\dist
OutputBaseFilename=Photo-Roulette-Setup-{#MyAppVersion}
SetupIconFile={#ProjectRoot}\assets\PhotoRoulette.ico
UninstallDisplayIcon={app}\PhotoRoulette.ico
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
VersionInfoVersion={#MyAppVersion}
LicenseFile={#ProjectRoot}\LICENSE
InfoBeforeFile={#ProjectRoot}\packaging\windows\BEFORE-INSTALL.txt

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Additional icons:"; Flags: unchecked

[Files]
Source: "{#ProjectRoot}\target\release\{#MyAppExeName}"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#ProjectRoot}\assets\PhotoRoulette.ico"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#ProjectRoot}\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#ProjectRoot}\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#ProjectRoot}\COMMERCIAL.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#ProjectRoot}\NOTICE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#ProjectRoot}\CLA.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#ProjectRoot}\assets\fonts\OFL-ArchivoBlack.txt"; DestDir: "{app}\licenses"; Flags: ignoreversion
Source: "{#ProjectRoot}\assets\fonts\OFL-SpaceGrotesk.txt"; DestDir: "{app}\licenses"; Flags: ignoreversion

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; IconFilename: "{app}\PhotoRoulette.ico"
Name: "{group}\Uninstall {#MyAppName}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; IconFilename: "{app}\PhotoRoulette.ico"; Tasks: desktopicon

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "Launch {#MyAppName}"; Flags: nowait postinstall skipifsilent

[UninstallDelete]
Type: filesandordirs; Name: "{app}\licenses"

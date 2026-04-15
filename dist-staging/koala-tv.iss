#define MyAppName "Koala TV"
#define MyAppVersion "1.0.0"
#define MyAppPublisher "Louis Delez"
#define MyAppURL "https://github.com/Louisdelez/KoalaTV"
#define MyAppExeName "koala-tv.exe"

[Setup]
AppId={{B5E3C7E9-4F21-4E8C-9B2A-KOALA-TV-V1}}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppVerName={#MyAppName} {#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}/issues
AppUpdatesURL={#MyAppURL}/releases
DefaultDirName={autopf}\KoalaTV
DefaultGroupName=Koala TV
DisableProgramGroupPage=yes
LicenseFile=koala-tv-v1.0.0-windows-x86_64\LICENSE
InfoAfterFile=koala-tv-v1.0.0-windows-x86_64\INSTALL.md
OutputDir=.
OutputBaseFilename=koala-tv-v1.0.0-windows-x86_64-setup
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
UninstallDisplayIcon={app}\{#MyAppExeName}

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "french"; MessagesFile: "compiler:Languages\French.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "koala-tv-v1.0.0-windows-x86_64\koala-tv.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "koala-tv-v1.0.0-windows-x86_64\libmpv-2.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "koala-tv-v1.0.0-windows-x86_64\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "koala-tv-v1.0.0-windows-x86_64\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "koala-tv-v1.0.0-windows-x86_64\INSTALL.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"
Name: "{group}\{cm:UninstallProgram,{#MyAppName}}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "{cm:LaunchProgram,{#StringChange(MyAppName, '&', '&&')}}"; Flags: nowait postinstall skipifsilent

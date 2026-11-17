; SPDX-License-Identifier: MPL-2.0
;
; Kinolune for Windows: the per-user installer (PKG-022, ADR-0109), built
; with Inno Setup 6 by packaging/app/windows/installer.sh from the folder of
; the portable zip (package.sh). No administrator rights: the app goes to
; %LOCALAPPDATA%\Programs\Kinolune, shortcuts to the user's Start menu and
; desktop, the uninstaller to Settings > Apps. The licences of the
; zip (licenses\, LGPL libmpv and FFmpeg included) are installed next to the
; app. Library, settings and logs live in %APPDATA%\app.kinolune and are
; kept on update and uninstall.
;
; The app updates itself by running a newer installer silently:
;   kinolune-<v>-windows-x64-setup.exe /VERYSILENT /SUPPRESSMSGBOXES /NORESTART /UPDATE=1 /RELAUNCH=1
; /UPDATE=1 waits (up to a minute) until the running app has quit;
; /RELAUNCH=1 starts the new version at the end.
;
; Defines (ISCC /D...): AppVersion (semver), AppVersionNumbers (a.b.c.d),
; SourceDir (the unpacked zip folder), OutputDir, SetupIcon (optional .ico).

#ifndef AppVersion
  #error AppVersion is not defined
#endif
#ifndef AppVersionNumbers
  #error AppVersionNumbers is not defined
#endif
#ifndef SourceDir
  #error SourceDir is not defined
#endif
#ifndef OutputDir
  #define OutputDir "."
#endif

; The single-instance mutex of the app (clients/flutter/app/windows/runner/main.cpp).
#define AppMutexName "Local\app.kinolune.Kinolune"

[Setup]
; Never change AppId: Windows finds the installed copy (and its uninstaller) by it.
AppId={{C3AAEA23-CF87-4B39-BFFF-855E4693C7BE}
AppName=Kinolune
AppVersion={#AppVersion}
AppVerName=Kinolune {#AppVersion}
AppPublisher=Kinolune
AppPublisherURL=https://github.com/DenisHumen/kinolune
AppSupportURL=https://github.com/DenisHumen/kinolune/issues
AppUpdatesURL=https://github.com/DenisHumen/kinolune/releases
VersionInfoVersion={#AppVersionNumbers}
VersionInfoProductVersion={#AppVersionNumbers}
VersionInfoProductTextVersion={#AppVersion}
DefaultDirName={localappdata}\Programs\Kinolune
DisableDirPage=yes
DefaultGroupName=Kinolune
DisableProgramGroupPage=yes
DisableWelcomePage=yes
DisableReadyPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputDir={#OutputDir}
OutputBaseFilename=kinolune-{#AppVersion}-windows-x64-setup
#ifdef SetupIcon
SetupIconFile={#SetupIcon}
#endif
UninstallDisplayIcon={app}\Kinolune.exe
UninstallDisplayName=Kinolune
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ShowLanguageDialog=no
LanguageDetectionMethod=uilanguage
; Files of a running Kinolune are closed through the Restart Manager; the
; app itself has normally quit already (/UPDATE=1).
CloseApplications=force
RestartApplications=no
SetupLogging=yes

[Languages]
Name: "ru"; MessagesFile: "compiler:Languages\Russian.isl"
Name: "en"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"

[InstallDelete]
; Assets and DLLs of the previous version: nothing stale stays behind.
Type: filesandordirs; Name: "{app}\data"
Type: files; Name: "{app}\*.dll"

[Files]
Source: "{#SourceDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\Kinolune"; Filename: "{app}\Kinolune.exe"
Name: "{autodesktop}\Kinolune"; Filename: "{app}\Kinolune.exe"; Tasks: desktopicon

[Run]
; An interactive install offers to start the app; a silent update started
; by the app starts it with /RELAUNCH=1.
Filename: "{app}\Kinolune.exe"; Description: "{cm:LaunchProgram,Kinolune}"; Flags: nowait postinstall skipifsilent
Filename: "{app}\Kinolune.exe"; Flags: nowait; Check: RelaunchRequested

[Code]
function RelaunchRequested: Boolean;
begin
  Result := ExpandConstant('{param:RELAUNCH|0}') = '1';
end;

function InitializeSetup: Boolean;
var
  Waited: Integer;
begin
  { An update started by the app: give it up to a minute to quit. }
  if ExpandConstant('{param:UPDATE|0}') = '1' then
  begin
    Waited := 0;
    while CheckForMutexes('{#AppMutexName}') and (Waited < 120) do
    begin
      Sleep(500);
      Waited := Waited + 1;
    end;
  end;
  Result := True;
end;

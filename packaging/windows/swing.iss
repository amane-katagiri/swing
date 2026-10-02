#ifndef AppVersion
  #error Pass /DAppVersion=<the version in Cargo.toml>
#endif
#ifndef StageDir
  #error Pass /DStageDir=<directory holding the files to install>
#endif
#ifndef RefName
  #define RefName "v" + AppVersion
#endif
#if Pos("-", AppVersion) > 0
  #define AppNumericVersion Copy(AppVersion, 1, Pos("-", AppVersion) - 1)
#else
  #define AppNumericVersion AppVersion
#endif

[Setup]
AppId={{E8A9B45D-3A72-492B-905A-51911FD534C2}
AppName=SWING
AppVersion={#AppVersion}
AppPublisher=Amane Katagiri
AppPublisherURL=https://github.com/amane-katagiri/swing
AppSupportURL=https://github.com/amane-katagiri/swing
AppUpdatesURL=https://github.com/amane-katagiri/swing/releases
VersionInfoVersion={#AppNumericVersion}
VersionInfoProductVersion={#AppNumericVersion}
VersionInfoProductTextVersion={#AppVersion}
PrivilegesRequired=lowest
DefaultDirName={autopf}\SWING
DisableProgramGroupPage=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.18362
ChangesEnvironment=yes
CloseApplications=no
RestartApplications=no
OutputBaseFilename=swing-{#RefName}-x86_64-pc-windows-msvc-setup
SetupIconFile=..\..\assets\swing.ico
UninstallDisplayIcon={app}\swing.exe
UninstallDisplayName=SWING
WizardStyle=modern
ShowLanguageDialog=auto
Compression=lzma2/ultra64
SolidCompression=yes

[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl"
Name: "ja"; MessagesFile: "compiler:Languages\Japanese.isl"

[Messages]
en.UninstalledAll=%1 was removed from your computer.%n%nYour settings, keys and IPFS data were kept in AppData\Local\swing in your user folder. Delete that folder if you no longer need them.
ja.UninstalledAll=%1 をアンインストールしました。%n%n設定・鍵・IPFS のデータは、ユーザーフォルダーの AppData\Local\swing に残してあります。不要ならこのフォルダーを削除してください。
en.UninstalledMost=%1 was removed, but some files could not be removed. These can be removed manually.%n%nYour settings, keys and IPFS data were kept in AppData\Local\swing in your user folder. Delete that folder if you no longer need them.
ja.UninstalledMost=%1 をアンインストールしましたが、削除できなかったファイルがあります。手動で削除してください。%n%n設定・鍵・IPFS のデータは、ユーザーフォルダーの AppData\Local\swing に残してあります。不要ならこのフォルダーを削除してください。

[CustomMessages]
en.StartSwing=Start SWING and open the dashboard
ja.StartSwing=SWING を起動してダッシュボードを開く
en.ServiceInstallFailed=SWING was installed, but registering it to start at sign-in failed (exit code %1).%n%nRun "swing service install" in a terminal to try again.
en.KeptRegistrations=These sign-in registrations start SWING from another folder, so they were left as they are:%n%n%1%nIf you no longer use that copy, run "swing service uninstall" from it.
ja.KeptRegistrations=次のサインイン時の起動の登録は別のフォルダーの SWING を起動するものなので、そのまま残しました。%n%n%1%nそのコピーを使っていなければ、そちらの「swing service uninstall」で登録を消してください。
ja.ServiceInstallFailed=SWING をインストールしましたが、サインイン時に起動する登録に失敗しました（終了コード %1）。%n%nターミナルで「swing service install」を実行してやり直してください。

[Files]
Source: "{#StageDir}\swing.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\swing-tray.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\ipfs.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\swing.example.toml"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\LICENSE-PixelMplus.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\LICENSE-Kubo.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\LICENSE-Kubo-APACHE.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\LICENSE-Kubo-MIT.txt"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\SWING"; Filename: "{app}\swing-tray.exe"; WorkingDir: "{app}"

[Run]
Filename: "{app}\swing.exe"; Parameters: "service start"; WorkingDir: "{app}"; Description: "{cm:StartSwing}"; Flags: postinstall skipifsilent runhidden waituntilterminated; Check: IsServiceRegistered; AfterInstall: StartTrayAndOpenDashboard

[Code]
const
  RunKey = 'Software\Microsoft\Windows\CurrentVersion\Run';
  EnvKey = 'Environment';

var
  IsUpgrade: Boolean;
  WasTaskRegistered: Boolean;
  WasTaskOurs: Boolean;
  WasTrayRegistered: Boolean;
  WasSwingUp: Boolean;
  WasTrayRunning: Boolean;
  ServiceRegistered: Boolean;
  KeptRegistrations: String;

function AppPath(const Name: String): String;
begin
  Result := ExpandConstant('{app}\' + Name);
end;

function WqlQuote(const S: String): String;
begin
  Result := S;
  StringChangeEx(Result, '\', '\\', True);
  StringChangeEx(Result, '''', '\''', True);
end;

function ProcessesOf(const Path, Extra: String): Variant;
var
  Locator, Service: Variant;
begin
  Locator := CreateOleObject('WbemScripting.SWbemLocator');
  Service := Locator.ConnectServer('.', 'root\CIMV2');
  Result := Service.ExecQuery('SELECT ProcessId FROM Win32_Process WHERE ExecutablePath = ''' + WqlQuote(Path) + '''' + Extra);
end;

function CountProcesses(const Path, Extra: String): Integer;
begin
  Result := 0;
  try
    Result := ProcessesOf(Path, Extra).Count;
  except
    Log('Could not list the processes of ' + Path + ': ' + GetExceptionMessage);
  end;
end;

procedure TerminateProcesses(const Path: String);
var
  Procs: Variant;
  I: Integer;
begin
  try
    Procs := ProcessesOf(Path, '');
    for I := 0 to Procs.Count - 1 do
    begin
      Log('Terminating a process of ' + Path);
      Procs.ItemIndex(I).Terminate(1);
    end;
  except
    Log('Could not terminate the processes of ' + Path + ': ' + GetExceptionMessage);
  end;
end;

function RunCaptured(const Exe, Params: String; var Output: TExecOutput): Integer;
begin
  try
    if not ExecAndCaptureOutput(Exe, Params, ExpandConstant('{app}'), SW_SHOWNORMAL, ewWaitUntilTerminated, Result, Output) then
    begin
      Log('Could not run ' + Exe + ' ' + Params + ': ' + SysErrorMessage(Result));
      Result := -1;
    end;
  except
    Log('Could not run ' + Exe + ' ' + Params + ': ' + GetExceptionMessage);
    Result := -1;
  end;
end;

function RunSwingCaptured(const Params: String; var Output: TExecOutput): Integer;
begin
  Result := RunCaptured(AppPath('swing.exe'), Params, Output);
end;

function AppDirArg: String;
begin
  Result := '"' + ExpandConstant('{app}') + '"';
end;

function CheckRegistrations(const Exe: String; var Outside: String): Integer;
var
  Output: TExecOutput;
  Params: String;
  I: Integer;
begin
  Params := 'service status --points-into ' + AppDirArg;
  Result := RunCaptured(Exe, Params, Output);
  Log(Exe + ' ' + Params + ' exited with ' + IntToStr(Result));
  Outside := '';
  for I := 0 to GetArrayLength(Output.StdOut) - 1 do
  begin
    Log(Output.StdOut[I]);
    if (Result = 4) and (Pos('which is under', Output.StdOut[I]) = 0) then
      Outside := Outside + Output.StdOut[I] + #13#10;
  end;
  for I := 0 to GetArrayLength(Output.StdErr) - 1 do
    Log(Output.StdErr[I]);
end;

function RunSwingLogged(const Params: String): Integer;
begin
  Log('Running swing.exe ' + Params);
  try
    if not ExecAndLogOutput(AppPath('swing.exe'), Params, ExpandConstant('{app}'), SW_SHOWNORMAL, ewWaitUntilTerminated, Result, nil) then
    begin
      Log('Could not run swing.exe ' + Params + ': ' + SysErrorMessage(Result));
      Result := -1;
      Exit;
    end;
  except
    Log('Could not run swing.exe ' + Params + ': ' + GetExceptionMessage);
    Result := -1;
    Exit;
  end;
  Log('swing.exe ' + Params + ' exited with ' + IntToStr(Result));
end;

function SwingRunning: Boolean;
begin
  Result := (CountProcesses(AppPath('swing.exe'), '') + CountProcesses(AppPath('ipfs.exe'), '')) > 0;
end;

function TrayRunning: Boolean;
begin
  Result := CountProcesses(AppPath('swing-tray.exe'), '') > 0;
end;

function WaitForSwingExit(const Seconds: Integer): Boolean;
var
  I: Integer;
begin
  for I := 1 to Seconds * 2 do
  begin
    if not SwingRunning then
    begin
      Result := True;
      Exit;
    end;
    Sleep(500);
  end;
  Result := not SwingRunning;
end;

function WaitForTrayExit(const Seconds: Integer): Boolean;
var
  I: Integer;
begin
  for I := 1 to Seconds * 2 do
  begin
    if not TrayRunning then
    begin
      Result := True;
      Exit;
    end;
    Sleep(500);
  end;
  Result := not TrayRunning;
end;

procedure StopEverything(const SwingCommand: String; const TraySeconds: Integer);
begin
  if SwingRunning then
  begin
    RunSwingLogged(SwingCommand);
    if not WaitForSwingExit(60) then
    begin
      TerminateProcesses(AppPath('swing.exe'));
      TerminateProcesses(AppPath('ipfs.exe'));
      WaitForSwingExit(10);
    end;
  end;
  if TrayRunning and not WaitForTrayExit(TraySeconds) then
  begin
    TerminateProcesses(AppPath('swing-tray.exe'));
    WaitForTrayExit(10);
  end;
end;

function TaskRegistered: Boolean;
var
  Code: Integer;
begin
  Result := Exec(ExpandConstant('{sys}\schtasks.exe'), '/Query /TN swing', '', SW_HIDE, ewWaitUntilTerminated, Code) and (Code = 0);
end;

function IsServiceRegistered: Boolean;
begin
  Result := ServiceRegistered;
end;

procedure StartTray;
var
  Cmd, Exe, Params: String;
  P, Code: Integer;
begin
  Exe := AppPath('swing-tray.exe');
  Params := '';
  if RegQueryStringValue(HKEY_CURRENT_USER, RunKey, 'swing-tray', Cmd) and (Length(Cmd) > 1) and (Cmd[1] = '"') then
  begin
    Delete(Cmd, 1, 1);
    P := Pos('"', Cmd);
    if (P > 0) and (CompareText(Copy(Cmd, 1, P - 1), Exe) = 0) then
      Params := Trim(Copy(Cmd, P + 1, MaxInt));
  end;
  if not Exec(Exe, Params, ExpandConstant('{app}'), SW_SHOWNORMAL, ewNoWait, Code) then
    Log('Could not start swing-tray.exe: ' + SysErrorMessage(Code));
end;

procedure OpenDashboard;
var
  Output: TExecOutput;
  Url: String;
  I, Code: Integer;
begin
  for I := 1 to 60 do
  begin
    if (RunSwingCaptured('dashboard open --no-browser', Output) = 0) and (GetArrayLength(Output.StdOut) > 0) then
    begin
      Url := Trim(Output.StdOut[0]);
      if (Pos('http://', Url) = 1) or (Pos('https://', Url) = 1) then
        ShellExec('open', Url, '', '', SW_SHOWNORMAL, ewNoWait, Code)
      else
        Log('swing.exe dashboard open printed no URL');
      Exit;
    end;
    Sleep(1000);
  end;
  Log('The dashboard did not come up within 60 seconds');
end;

procedure StartTrayAndOpenDashboard;
begin
  StartTray;
  OpenDashboard;
end;

function SameDir(const A, B: String): Boolean;
begin
  Result := CompareText(RemoveBackslashUnlessRoot(Trim(A)), RemoveBackslashUnlessRoot(Trim(B))) = 0;
end;

procedure AddToPath(const Dir: String);
var
  Path, Rest: String;
  P: Integer;
begin
  if not RegQueryStringValue(HKEY_CURRENT_USER, EnvKey, 'Path', Path) then
    Path := '';
  Rest := Path + ';';
  while Rest <> '' do
  begin
    P := Pos(';', Rest);
    if SameDir(Copy(Rest, 1, P - 1), Dir) then
      Exit;
    Delete(Rest, 1, P);
  end;
  if (Path <> '') and (Path[Length(Path)] <> ';') then
    Path := Path + ';';
  if not RegWriteExpandStringValue(HKEY_CURRENT_USER, EnvKey, 'Path', Path + Dir) then
    Log('Could not add ' + Dir + ' to the user PATH');
end;

procedure RemoveFromPath(const Dir: String);
var
  Path, Rest, Entry, NewPath: String;
  P: Integer;
  Found: Boolean;
begin
  if not RegQueryStringValue(HKEY_CURRENT_USER, EnvKey, 'Path', Path) then
    Exit;
  Rest := Path + ';';
  NewPath := '';
  Found := False;
  while Rest <> '' do
  begin
    P := Pos(';', Rest);
    Entry := Copy(Rest, 1, P - 1);
    Delete(Rest, 1, P);
    if SameDir(Entry, Dir) then
      Found := True
    else if Entry <> '' then
    begin
      if NewPath <> '' then
        NewPath := NewPath + ';';
      NewPath := NewPath + Entry;
    end;
  end;
  if not Found then
    Exit;
  if NewPath = '' then
    RegDeleteValue(HKEY_CURRENT_USER, EnvKey, 'Path')
  else if not RegWriteExpandStringValue(HKEY_CURRENT_USER, EnvKey, 'Path', NewPath) then
    Log('Could not remove ' + Dir + ' from the user PATH');
end;

procedure RegisterService(const NoTray: Boolean);
var
  Params: String;
  Code: Integer;
begin
  Params := 'service install --no-start';
  if NoTray then
    Params := Params + ' --no-tray';
  Code := RunSwingLogged(Params);
  ServiceRegistered := Code = 0;
  if not ServiceRegistered then
    SuppressibleMsgBox(FmtMessage(CustomMessage('ServiceInstallFailed'), [IntToStr(Code)]), mbError, MB_OK, IDOK);
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Outside: String;
begin
  Result := '';
  IsUpgrade := FileExists(AppPath('swing.exe'));
  if not IsUpgrade then
    Exit;
  WasTaskRegistered := TaskRegistered;
  WasTaskOurs := False;
  if WasTaskRegistered then
  begin
    ExtractTemporaryFile('swing.exe');
    WasTaskOurs := CheckRegistrations(ExpandConstant('{tmp}\swing.exe'), Outside) = 0;
  end;
  WasTrayRegistered := RegValueExists(HKEY_CURRENT_USER, RunKey, 'swing-tray');
  WasSwingUp := WasTaskRegistered and (CountProcesses(AppPath('swing.exe'), ' AND (CommandLine LIKE ''% up %'' OR CommandLine LIKE ''% up'')') > 0);
  WasTrayRunning := TrayRunning;
  Log(Format('Upgrading: task registered=%d, tray registered=%d, swing up running=%d, tray running=%d, registrations point here=%d', [Ord(WasTaskRegistered), Ord(WasTrayRegistered), Ord(WasSwingUp), Ord(WasTrayRunning), Ord(WasTaskOurs)]));
  if WasTaskRegistered and not WasTaskOurs then
    StopEverything('stop', 0)
  else
    StopEverything('service stop', 0);
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep <> ssPostInstall then
    Exit;
  AddToPath(ExpandConstant('{app}'));
  if IsUpgrade and not WasTaskRegistered then
    Log('The service was not registered before the upgrade; leaving it unregistered')
  else if IsUpgrade and not WasTaskOurs then
    Log('The registrations do not all start swing from ' + ExpandConstant('{app}') + '; leaving them as they are')
  else
    RegisterService(IsUpgrade and not WasTrayRegistered);
  if ServiceRegistered and WasSwingUp then
  begin
    RunSwingLogged('service start');
    if WasTrayRunning then
      StartTray;
  end;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Outside: String;
begin
  if CurUninstallStep = usUninstall then
  begin
    KeptRegistrations := '';
    if FileExists(AppPath('swing.exe')) then
    begin
      if CheckRegistrations(AppPath('swing.exe'), Outside) = 4 then
        KeptRegistrations := Outside;
      RunSwingLogged('service uninstall --only-from ' + AppDirArg);
    end;
    if KeptRegistrations <> '' then
      StopEverything('stop', 0)
    else
      StopEverything('service stop', 30);
    RemoveFromPath(ExpandConstant('{app}'));
  end
  else if (CurUninstallStep = usPostUninstall) and (KeptRegistrations <> '') and not UninstallSilent then
    MsgBox(FmtMessage(CustomMessage('KeptRegistrations'), [KeptRegistrations]), mbInformation, MB_OK);
end;

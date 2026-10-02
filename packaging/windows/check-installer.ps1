param(
  [Parameter(Mandatory)] [ValidateSet('install', 'upgrade', 'uninstall', 'foreign')] [string] $Phase,
  [Parameter(Mandatory)] [string] $Setup,
  [Parameter(Mandatory)] [string] $OutDir
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$App = Join-Path $env:LOCALAPPDATA 'Programs\SWING'
$Other = Join-Path $env:LOCALAPPDATA 'swing-check-other-copy'
$Data = Join-Path $env:LOCALAPPDATA 'swing'
$Config = Join-Path $Data 'swing.toml'
$RunKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$UninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\{E8A9B45D-3A72-492B-905A-51911FD534C2}_is1'
$Shortcut = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\SWING.lnk'
$Files = @(
  'swing.exe', 'swing-tray.exe', 'ipfs.exe', 'swing.example.toml', 'README.md', 'LICENSE',
  'LICENSE-PixelMplus.txt', 'LICENSE-Kubo.txt', 'LICENSE-Kubo-APACHE.txt', 'LICENSE-Kubo-MIT.txt', 'unins000.exe'
)
$Silent = @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART')

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
Set-Location $env:USERPROFILE

function Check([bool] $Ok, [string] $What) {
  if (-not $Ok) { throw "failed: $What" }
  "ok: $What"
}

function Save([string] $Name, $Value) {
  $Value | Out-String | Out-File -Encoding utf8 (Join-Path $OutDir "$Phase-$Name.txt")
}

function AppPathEntries {
  $path = (Get-ItemProperty 'HKCU:\Environment' -Name Path -ErrorAction SilentlyContinue)
  if (-not $path) { return @() }
  @($path.Path -split ';' | Where-Object { $_.TrimEnd('\') -ieq $App })
}

function TaskRegistered {
  & schtasks.exe /Query /TN swing *> $null
  $LASTEXITCODE -eq 0
}

function TaskXml {
  (& schtasks.exe /Query /TN swing /XML 2>$null) -join "`n"
}

function Mentions([string] $Text, [string] $Path) {
  $null -ne $Text -and $Text.IndexOf($Path, [StringComparison]::OrdinalIgnoreCase) -ge 0
}

function WaitUninstalled {
  for ($i = 0; $i -lt 180; $i++) {
    $second = @(Get-CimInstance Win32_Process -Filter "Name LIKE '[_]iu%.tmp' OR Name = '_unins.tmp'")
    if (-not (Test-Path $UninstallKey) -and @($second).Count -eq 0 -and -not (Test-Path (Join-Path $App 'unins000.exe'))) { break }
    Start-Sleep 1
  }
}

function RunValue {
  $v = Get-ItemProperty $RunKey -Name 'swing-tray' -ErrorAction SilentlyContinue
  if ($v) { $v.'swing-tray' } else { $null }
}

function Ours([string] $Name) {
  @(Get-CimInstance Win32_Process -Filter "Name = '$Name'" | Where-Object { $_.ExecutablePath -and $_.ExecutablePath.StartsWith($App + '\', 'OrdinalIgnoreCase') })
}

function Running { @(Ours 'swing.exe') + @(Ours 'swing-tray.exe') + @(Ours 'ipfs.exe') }

function Swing {
  & (Join-Path $App 'swing.exe') @args 2>&1 | ForEach-Object { "$_" }
}

function WaitDashboard {
  for ($i = 0; $i -lt 60; $i++) {
    $null = Swing dashboard open --no-browser
    if ($LASTEXITCODE -eq 0) { return $true }
    Start-Sleep 1
  }
  $false
}

function RunSetup([string] $Exe, [string] $Log) {
  $p = Start-Process -FilePath $Exe -ArgumentList ($Silent + "/LOG=`"$Log`"") -PassThru
  # Start-Process -Wait also waits for swing and the tray that an upgrade starts again.
  $p.WaitForExit()
  $p.ExitCode
}

function Snapshot {
  Save processes (Get-CimInstance Win32_Process | Where-Object { $_.Name -match '^(swing|swing-tray|ipfs|conhost)\.exe$' } | Select-Object ProcessId, ParentProcessId, Name, CommandLine | Format-List)
  Save run-key (Get-ItemProperty $RunKey -ErrorAction SilentlyContinue | Format-List)
  Save user-path ((Get-ItemProperty 'HKCU:\Environment' -Name Path -ErrorAction SilentlyContinue | Format-List))
  Save task (schtasks.exe /Query /TN swing /FO LIST /V 2>&1)
  if (Test-Path $Data) { Save data-dir (Get-ChildItem -Force -Recurse -Depth 1 $Data | Select-Object FullName, Length) }
  if (Test-Path (Join-Path $Data 'swing.log')) { Copy-Item (Join-Path $Data 'swing.log') (Join-Path $OutDir "$Phase-swing.log") }
}

try {
  switch ($Phase) {
    'install' {
      Check (-not (Test-Path $App)) 'nothing is installed before the check'
      Check (-not (Test-Path $Data)) 'there is no data directory before the check'
      $code = RunSetup $Setup (Join-Path $OutDir 'install.log')
      Check ($code -eq 0) "silent install exits with 0 (got $code)"
      foreach ($f in $Files) { Check (Test-Path (Join-Path $App $f)) "$f is installed" }
      Check (@(AppPathEntries).Count -eq 1) 'the install directory is on the user PATH once'
      Check (TaskRegistered) 'the swing task is registered'
      $run = RunValue
      Check ($null -ne $run -and $run.IndexOf("$App\swing-tray.exe", [StringComparison]::OrdinalIgnoreCase) -ge 0) 'the tray Run value points to the installed swing-tray.exe'
      Check (Test-Path $Config) 'an empty swing.toml is created at the per-user default location'
      Check ((Get-Item $Config).Length -eq 0) 'the created swing.toml is empty'
      Check (Test-Path $Shortcut) 'the Start menu shortcut exists'
      Check (Test-Path $UninstallKey) 'the uninstall entry exists'
      Start-Sleep 5
      Check (@(Running).Count -eq 0) 'nothing from the install directory runs after a silent install'
    }
    'upgrade' {
      $null = Swing service start
      Check ($LASTEXITCODE -eq 0) 'swing service start succeeds'
      Check (WaitDashboard) 'swing up answers before the upgrade'
      Start-Process -FilePath (Join-Path $App 'swing-tray.exe') -WorkingDirectory $App
      Start-Sleep 8
      Check (@(Ours 'swing-tray.exe').Count -gt 0) 'swing-tray runs before the upgrade'
      $before = @(Ours 'swing.exe' | ForEach-Object { $_.ProcessId })
      Snapshot
      $log = Join-Path $OutDir 'upgrade.log'
      $code = RunSetup $Setup $log
      Check ($code -eq 0) "silent upgrade exits with 0 (got $code)"
      $text = Get-Content -Raw $log
      Check ($text -match 'Upgrading: task registered=1, tray registered=1, swing up running=1, tray running=1, registrations point here=1') 'the upgrade sees the running service and tray as its own'
      Check ($text -notmatch 'Terminating a process of .*\\swing\.exe') 'swing stopped without being terminated'
      Check (WaitDashboard) 'swing up runs again after the upgrade'
      $after = @(Ours 'swing.exe' | ForEach-Object { $_.ProcessId })
      Check (@($after | Where-Object { $before -contains $_ }).Count -eq 0) 'swing up was restarted by the upgrade'
      Start-Sleep 5
      Check (@(Ours 'swing-tray.exe').Count -gt 0) 'swing-tray runs again after the upgrade'
      Check (TaskRegistered) 'the swing task is still registered'
      Check ($null -ne (RunValue)) 'the tray Run value is still there'
      Check (@(AppPathEntries).Count -eq 1) 'the install directory is still on the user PATH once'
    }
    'uninstall' {
      $log = Join-Path $OutDir 'uninstall.log'
      $code = RunSetup (Join-Path $App 'unins000.exe') $log
      Check ($code -eq 0) "silent uninstall exits with 0 (got $code)"
      WaitUninstalled
      Check (-not (Test-Path $UninstallKey)) 'the uninstall entry is removed'
      $text = Get-Content -Raw $log
      Check ($text -notmatch 'Terminating a process of .*\\swing\.exe') 'swing stopped without being terminated'
      Save tray-terminated ([bool]($text -match 'Terminating a process of .*\\swing-tray\.exe'))
      Check (-not (TaskRegistered)) 'the swing task is removed'
      Check ($null -eq (RunValue)) 'the tray Run value is removed'
      Check (@(AppPathEntries).Count -eq 0) 'the install directory is removed from the user PATH'
      foreach ($f in $Files) { Check (-not (Test-Path (Join-Path $App $f))) "$f is removed" }
      Check (-not (Test-Path $Shortcut)) 'the Start menu shortcut is removed'
      Check (@(Running).Count -eq 0) 'nothing from the install directory runs after the uninstall'
      Check (Test-Path $Config) 'swing.toml in the data directory is kept'
    }
    'foreign' {
      Check (-not (Test-Path (Join-Path $App 'swing.exe'))) 'nothing is installed before the check'
      try {
        $code = RunSetup $Setup (Join-Path $OutDir 'foreign-install.log')
        Check ($code -eq 0) "silent install exits with 0 (got $code)"
        New-Item -ItemType Directory -Force -Path $Other | Out-Null
        Copy-Item (Join-Path $App 'swing.exe'), (Join-Path $App 'swing-tray.exe') $Other
        $out = & (Join-Path $Other 'swing.exe') service install --no-start 2>&1 | ForEach-Object { "$_" }
        Save other-install $out
        Check ($LASTEXITCODE -eq 0) 'swing service install from another copy succeeds'
        Check (Mentions (TaskXml) "$Other\swing.exe") 'the task now starts the other copy'
        Check (Mentions (RunValue) "$Other\swing-tray.exe") 'the tray Run value now starts the other copy'

        $log = Join-Path $OutDir 'foreign-upgrade.log'
        $code = RunSetup $Setup $log
        Check ($code -eq 0) "silent upgrade exits with 0 (got $code)"
        $text = Get-Content -Raw $log
        Check ($text -match 'registrations point here=0') 'the upgrade sees that the registrations are not its own'
        Check (Mentions (TaskXml) "$Other\swing.exe") 'the upgrade leaves the task of the other copy'
        Check (Mentions (RunValue) "$Other\swing-tray.exe") 'the upgrade leaves the tray Run value of the other copy'

        $log = Join-Path $OutDir 'foreign-uninstall.log'
        $code = RunSetup (Join-Path $App 'unins000.exe') $log
        Check ($code -eq 0) "silent uninstall exits with 0 (got $code)"
        WaitUninstalled
        Check (-not (Test-Path $UninstallKey)) 'the uninstall entry is removed'
        $text = Get-Content -Raw $log
        Check ($text -match 'left it as is') 'the uninstaller reports the registrations it kept'
        Check (TaskRegistered) 'the task of the other copy is kept'
        Check (Mentions (TaskXml) "$Other\swing.exe") 'the kept task still starts the other copy'
        Check (Mentions (RunValue) "$Other\swing-tray.exe") 'the tray Run value of the other copy is kept'
        foreach ($f in $Files) { Check (-not (Test-Path (Join-Path $App $f))) "$f is removed" }
        Check (@(Running).Count -eq 0) 'nothing from the install directory runs after the uninstall'
      } finally {
        if (Test-Path (Join-Path $Other 'swing.exe')) {
          Save other-uninstall (& (Join-Path $Other 'swing.exe') service uninstall 2>&1 | ForEach-Object { "$_" })
        }
        Remove-Item -Recurse -Force $Other -ErrorAction SilentlyContinue
      }
      Check (-not (TaskRegistered)) 'the task of the other copy is cleaned up'
      Check ($null -eq (RunValue)) 'the tray Run value of the other copy is cleaned up'
    }
  }
} finally {
  Snapshot
}
# Otherwise the runner exits with the code of the last native command, such as an expected schtasks failure.
exit 0

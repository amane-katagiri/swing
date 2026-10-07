param(
  [Parameter(Mandatory)] [string] $Archive,
  [Parameter(Mandatory)] [string] $OutDir,
  [Parameter(Mandatory)] [string] $RefName,
  [string] $WorkDir = (Join-Path ([IO.Path]::GetTempPath()) 'swing-installer'),
  [string] $Iscc
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
Set-StrictMode -Version Latest

$InnoVersion = '6.7.3'
$InnoSha256 = '9c73c3bae7ed48d44112a0f48e66742c00090bdb5bef71d9d3c056c66e97b732'

$Root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path

function Get-FirstMatch([string] $Path, [string] $Pattern) {
  $m = Select-String -Path $Path -Pattern $Pattern | Select-Object -First 1
  if (-not $m) { throw "no line matching $Pattern in $Path" }
  $m.Matches[0].Groups[1].Value
}

function Assert-Hash([string] $Path, [string] $Algorithm, [string] $Expected) {
  $actual = (Get-FileHash -Algorithm $Algorithm -LiteralPath $Path).Hash.ToLowerInvariant()
  if ($actual -ne $Expected.ToLowerInvariant()) { throw "$Algorithm of $Path is $actual, expected $Expected" }
}

function Read-Checksum([string] $Path) {
  $fields = @((Get-Content -Raw -LiteralPath $Path).Trim() -split '\s+')
  if ($fields.Count -ne 2) { throw "$Path is not a '<hash>  <file name>' line" }
  $fields
}

$version = Get-FirstMatch (Join-Path $Root 'Cargo.toml') '^version\s*=\s*"([^"]+)"'
$kuboVersion = Get-FirstMatch (Join-Path $Root 'src\kubo\mod.rs') 'KUBO_VERSION: &str = "([^"]+)"'
$kuboZip = "kubo_v${kuboVersion}_windows-amd64.zip"
$pinned = Read-Checksum (Join-Path $PSScriptRoot 'kubo.sha512')
if ($pinned[1] -ne $kuboZip) { throw "packaging/windows/kubo.sha512 pins $($pinned[1]), but src/kubo/mod.rs wants $kuboZip" }

if (Test-Path $WorkDir) { Remove-Item -Recurse -Force $WorkDir }
$download = New-Item -ItemType Directory -Path (Join-Path $WorkDir 'download')
$stage = New-Item -ItemType Directory -Path (Join-Path $WorkDir 'stage')
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

$kuboBase = "https://github.com/ipfs/kubo/releases/download/v$kuboVersion"
$kuboPath = Join-Path $download $kuboZip
Invoke-WebRequest -MaximumRetryCount 5 -RetryIntervalSec 10 -Uri "$kuboBase/$kuboZip" -OutFile $kuboPath
Invoke-WebRequest -MaximumRetryCount 5 -RetryIntervalSec 10 -Uri "$kuboBase/$kuboZip.sha512" -OutFile "$kuboPath.sha512"
$published = Read-Checksum "$kuboPath.sha512"
if ($published[0] -ne $pinned[0] -or $published[1] -ne $kuboZip) { throw "the checksum published at $kuboBase differs from packaging/windows/kubo.sha512" }
Assert-Hash $kuboPath 'SHA512' $pinned[0]

Expand-Archive -LiteralPath $kuboPath -DestinationPath (Join-Path $WorkDir 'kubo')
$kubo = Join-Path $WorkDir 'kubo\kubo'
Copy-Item (Join-Path $kubo 'ipfs.exe') $stage
Copy-Item (Join-Path $kubo 'LICENSE') (Join-Path $stage 'LICENSE-Kubo.txt')
Copy-Item (Join-Path $kubo 'LICENSE-APACHE') (Join-Path $stage 'LICENSE-Kubo-APACHE.txt')
Copy-Item (Join-Path $kubo 'LICENSE-MIT') (Join-Path $stage 'LICENSE-Kubo-MIT.txt')

$unpacked = Join-Path $WorkDir 'swing'
Expand-Archive -LiteralPath $Archive -DestinationPath $unpacked
$inner = @(Get-ChildItem -Directory -LiteralPath $unpacked)
if ($inner.Count -ne 1) { throw "$Archive should hold exactly one directory" }
Copy-Item (Join-Path $inner[0].FullName '*') $stage

if (-not $Iscc) {
  $installer = Join-Path $download "innosetup-$InnoVersion.exe"
  $tag = 'is-' + ($InnoVersion -replace '\.', '_')
  Invoke-WebRequest -MaximumRetryCount 5 -RetryIntervalSec 10 -Uri "https://github.com/jrsoftware/issrc/releases/download/$tag/innosetup-$InnoVersion.exe" -OutFile $installer
  Assert-Hash $installer 'SHA256' $InnoSha256
  $innoDir = Join-Path $WorkDir 'inno'
  $p = Start-Process -FilePath $installer -Wait -PassThru -ArgumentList @(
    '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/SP-', '/PORTABLE=1', '/CURRENTUSER', "/DIR=`"$innoDir`""
  )
  if ($p.ExitCode -ne 0) { throw "the Inno Setup installer exited with $($p.ExitCode)" }
  $Iscc = Join-Path $innoDir 'ISCC.exe'
}

& $Iscc "/DAppVersion=$version" "/DRefName=$RefName" "/DStageDir=$($stage.FullName)" "/O$((Resolve-Path $OutDir).Path)" (Join-Path $PSScriptRoot 'swing.iss')
if ($LASTEXITCODE -ne 0) { throw "ISCC exited with $LASTEXITCODE" }

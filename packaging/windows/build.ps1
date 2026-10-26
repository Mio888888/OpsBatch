# Build dist\OpsBatch-<version>-windows-<arch>-setup.exe (Inno Setup 6 required: https://jrsoftware.org/isinfo.php).
#
#   powershell -File packaging\windows\build.ps1
#
# Optional environment: OPSBATCH_FEATURES (real-vault for production), WINDOWS_CODESIGN_CERT_PATH /
# WINDOWS_CODESIGN_CERT_PASSWORD or WINDOWS_CODESIGN_CERT_THUMBPRINT (Authenticode; unsigned when absent).
$ErrorActionPreference = 'Stop'
$root = Resolve-Path (Join-Path $PSScriptRoot '..\..')
$version = if ($env:OPSBATCH_VERSION) { $env:OPSBATCH_VERSION } else { (Select-String -Path (Join-Path $root 'Cargo.toml') -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1).Matches[0].Groups[1].Value }
$arch = if ($env:PROCESSOR_ARCHITECTURE -eq 'ARM64') { 'aarch64' } else { 'x86_64' }
$dist = Join-Path $root 'dist'
New-Item -ItemType Directory -Force -Path $dist | Out-Null

$features = @()
if ($env:OPSBATCH_FEATURES) { $features = @('--features', $env:OPSBATCH_FEATURES) }
else { Write-Warning 'Building WITHOUT the real-vault feature - secrets are not persisted and the app uses the isolated -dev data directory.' }

Push-Location $root
try {
  cargo build --release -p opsbatch @features
  if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }
} finally { Pop-Location }

$exe = Join-Path $root 'target\release\opsbatch.exe'
& (Join-Path $PSScriptRoot 'sign.ps1') $exe

$iscc = (Get-Command iscc.exe -ErrorAction SilentlyContinue).Source
if (-not $iscc) { $iscc = Join-Path ${env:ProgramFiles(x86)} 'Inno Setup 6\ISCC.exe' }
if (-not (Test-Path $iscc)) { throw 'ISCC.exe (Inno Setup 6) not found' }
& $iscc "/DVersion=$version" "/DArch=$arch" "/DSourceDir=$(Join-Path $root 'target\release')" "/DOutputDir=$dist" (Join-Path $PSScriptRoot 'opsbatch.iss')
if ($LASTEXITCODE -ne 0) { throw 'iscc failed' }

$installer = Join-Path $dist "OpsBatch-$version-windows-$arch-setup.exe"
& (Join-Path $PSScriptRoot 'sign.ps1') $installer
Write-Output $installer

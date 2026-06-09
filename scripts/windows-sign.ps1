param(
  [Parameter(ValueFromRemainingArguments = $true)]
  [string[]]$Path,

  [string]$CertificatePath = $env:WINDOWS_CODESIGN_CERT_PATH,
  [string]$CertificatePassword = $env:WINDOWS_CODESIGN_CERT_PASSWORD,
  [string]$CertificateThumbprint = $env:WINDOWS_CODESIGN_CERT_THUMBPRINT,
  [string]$TimestampUrl = $(if ($env:WINDOWS_CODESIGN_TIMESTAMP_URL) { $env:WINDOWS_CODESIGN_TIMESTAMP_URL } else { 'http://timestamp.digicert.com' }),
  [string]$DigestAlgorithm = $(if ($env:WINDOWS_CODESIGN_DIGEST_ALGORITHM) { $env:WINDOWS_CODESIGN_DIGEST_ALGORITHM } else { 'sha256' }),
  [string]$SignTool = $env:SIGNTOOL_PATH
)

$ErrorActionPreference = 'Stop'

function Resolve-SignTool {
  param([string]$ConfiguredPath)

  if ($ConfiguredPath -and (Test-Path -LiteralPath $ConfiguredPath)) {
    return (Resolve-Path -LiteralPath $ConfiguredPath).Path
  }

  $command = Get-Command signtool.exe -ErrorAction SilentlyContinue
  if ($command) {
    return $command.Source
  }

  $kitsRoot = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'
  if (Test-Path -LiteralPath $kitsRoot) {
    $candidate = Get-ChildItem -LiteralPath $kitsRoot -Recurse -Filter signtool.exe -ErrorAction SilentlyContinue |
      Where-Object { $_.FullName -match '\\x64\\signtool\.exe$' } |
      Sort-Object FullName -Descending |
      Select-Object -First 1
    if ($candidate) {
      return $candidate.FullName
    }
  }

  throw 'signtool.exe not found. Install Windows SDK or set SIGNTOOL_PATH.'
}

function Resolve-SignTargets {
  param([string[]]$InputPaths)

  $targets = @()
  foreach ($item in $InputPaths) {
    if (-not $item) { continue }
    $expanded = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($item)
    if (Test-Path -LiteralPath $expanded -PathType Container) {
      $targets += Get-ChildItem -LiteralPath $expanded -Recurse -Include *.exe,*.msi | Select-Object -ExpandProperty FullName
    } elseif (Test-Path -LiteralPath $expanded -PathType Leaf) {
      $targets += $expanded
    } else {
      throw "Sign target not found: $item"
    }
  }

  if ($targets.Count -eq 0) {
    $bundleDir = Join-Path $PSScriptRoot '..\src-tauri\target\release\bundle'
    if (Test-Path -LiteralPath $bundleDir) {
      $targets += Get-ChildItem -LiteralPath $bundleDir -Recurse -Include *.exe,*.msi | Select-Object -ExpandProperty FullName
    }
  }

  $targets | Sort-Object -Unique
}

if ($CertificatePath) {
  if (-not (Test-Path -LiteralPath $CertificatePath)) {
    throw "Certificate file not found: $CertificatePath"
  }
} elseif ($CertificateThumbprint) {
  # Certificate store signing is configured below.
} else {
  Write-Warning 'Windows Authenticode signing skipped: set WINDOWS_CODESIGN_CERT_PATH or WINDOWS_CODESIGN_CERT_THUMBPRINT to sign release artifacts.'
  if ($env:GITHUB_ACTIONS -eq 'true') {
    Write-Host '::warning title=Windows signing skipped::Set WINDOWS_CODESIGN_CERT_PATH or WINDOWS_CODESIGN_CERT_THUMBPRINT to sign release artifacts.'
  }
  exit 0
}

$resolvedSignTool = Resolve-SignTool -ConfiguredPath $SignTool
$targets = @(Resolve-SignTargets -InputPaths $Path)

if ($targets.Count -eq 0) {
  throw 'No .exe or .msi targets found to sign.'
}

$signArgs = @('sign', '/fd', $DigestAlgorithm, '/tr', $TimestampUrl, '/td', $DigestAlgorithm)

if ($CertificatePath) {
  $signArgs += @('/f', (Resolve-Path -LiteralPath $CertificatePath).Path)
  if ($CertificatePassword) {
    $signArgs += @('/p', $CertificatePassword)
  }
} elseif ($CertificateThumbprint) {
  $signArgs += @('/sha1', $CertificateThumbprint)
}

foreach ($target in $targets) {
  Write-Host "Signing $target"
  & $resolvedSignTool @signArgs $target
  if ($LASTEXITCODE -ne 0) {
    throw "signtool failed for $target"
  }
}

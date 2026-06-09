param(
  [Parameter(ValueFromRemainingArguments = $true)]
  [string[]]$Path
)

$ErrorActionPreference = 'Stop'

$rootScript = Join-Path $PSScriptRoot '..\..\scripts\windows-sign.ps1'
if (-not (Test-Path -LiteralPath $rootScript)) {
  throw 'Root scripts/windows-sign.ps1 not found.'
}

& $rootScript @Path

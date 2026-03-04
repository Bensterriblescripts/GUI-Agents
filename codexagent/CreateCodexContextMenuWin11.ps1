if (-not ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
  Start-Process powershell.exe -Verb RunAs -ArgumentList '-NoProfile','-ExecutionPolicy','Bypass','-File',('"{0}"'-f$PSCommandPath); exit
}

$ErrorActionPreference = 'Stop'

$packageName = 'CodexAgent.ContextMenu'
$manifestPath = Join-Path $PSScriptRoot 'packaging\win11\AppxManifest.xml'
$installDir = 'C:\Local\Software'
$dllPath = Join-Path $installDir 'codexagent_contextmenu.dll'
$exePath = Join-Path $installDir 'codexagent.exe'
$legacyMenuName = 'Launch Codex'
$legacyMenuPaths = @(
  "HKLM:\Software\Classes\Directory\shell\$legacyMenuName",
  "HKLM:\Software\Classes\Directory\Background\shell\$legacyMenuName"
)
$legacyWin11Hack = 'HKCU:\Software\Classes\CLSID\{86ca1aa0-34aa-4e8b-a509-50c905bae2a2}'

if (-not (Test-Path $manifestPath)) {
  throw "Missing manifest: $manifestPath"
}
if (-not (Test-Path $exePath)) {
  throw "Missing executable: $exePath. Run .\Build.ps1 first."
}
if (-not (Test-Path $dllPath)) {
  throw "Missing shell extension: $dllPath. Run .\Build.ps1 first."
}

$package = Get-AppxPackage -Name $packageName -ErrorAction SilentlyContinue
if ($package) {
  Remove-AppxPackage -Package $package.PackageFullName
}

foreach ($path in $legacyMenuPaths) {
  Remove-Item -Path $path -Recurse -Force -ErrorAction SilentlyContinue
}
Remove-Item -Path $legacyWin11Hack -Recurse -Force -ErrorAction SilentlyContinue

Add-AppxPackage -Register $manifestPath -ExternalLocation $installDir -DisableDevelopmentMode -ForceApplicationShutdown

Stop-Process -Name explorer -Force

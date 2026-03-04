if (-not ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
  Start-Process powershell.exe -Verb RunAs -ArgumentList '-NoProfile','-ExecutionPolicy','Bypass','-File',('"{0}"'-f$PSCommandPath); exit
}

$ErrorActionPreference = 'Stop'

$packageName = 'CodexAgent.ContextMenu'
$legacyWin11Hack = 'HKCU:\Software\Classes\CLSID\{86ca1aa0-34aa-4e8b-a509-50c905bae2a2}'

$package = Get-AppxPackage -Name $packageName -ErrorAction SilentlyContinue
if ($package) {
  Remove-AppxPackage -Package $package.PackageFullName
}

Remove-Item -Path $legacyWin11Hack -Recurse -Force -ErrorAction SilentlyContinue

Stop-Process -Name explorer -Force

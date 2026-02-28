if (-not ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
  Start-Process powershell.exe -Verb RunAs -ArgumentList '-NoProfile','-ExecutionPolicy','Bypass','-File',('"{0}"'-f$PSCommandPath); exit
}

$MenuName = 'Launch Agent'
@(
  "HKLM:\Software\Classes\Directory\shell\$MenuName",
  "HKLM:\Software\Classes\Directory\Background\shell\$MenuName"
) | ForEach-Object {
  Remove-Item -Path $_ -Recurse -Force -ErrorAction SilentlyContinue
}

if (-not ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
  Start-Process powershell.exe -Verb RunAs -ArgumentList '-NoProfile','-ExecutionPolicy','Bypass','-File',('"{0}"'-f$PSCommandPath); exit
}

reg.exe add "HKCU\Software\Classes\CLSID\{86ca1aa0-34aa-4e8b-a509-50c905bae2a2}\InprocServer32" /ve /d "" /f

$n='Launch Agent';$exe='C:\Local\Software\autoagent.exe';$ico="$env:WINDIR\System32\imageres.dll,-109"

foreach($p in @('Directory,%1','Directory\Background,%V')){
  $cls,$tok=$p-split','
  $k="HKLM:\Software\Classes\$cls\shell\$n"
  New-Item "$k\command" -Force|Out-Null
  Set-ItemProperty $k '(Default)' $n
  Set-ItemProperty $k 'Icon' $ico
  Set-ItemProperty "$k\command" '(Default)' "`"$exe`" --show --cwd `"$tok`""
}
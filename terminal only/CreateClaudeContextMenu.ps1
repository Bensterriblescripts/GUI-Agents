if (-not ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
  Start-Process powershell.exe -Verb RunAs -ArgumentList '-NoProfile','-ExecutionPolicy','Bypass','-File',('"{0}"'-f$PSCommandPath); exit
}

reg.exe add "HKCU\Software\Classes\CLSID\{86ca1aa0-34aa-4e8b-a509-50c905bae2a2}\InprocServer32" /ve /d "" /f

$MenuName = 'Launch Claude'
$MenuText = 'Launch Claude'
$WtExe = (Get-Command wt.exe -ErrorAction Stop).Source
$ClaudeExe = (Get-Command claude.exe, claude -ErrorAction Stop | Select-Object -First 1).Source
$IconPath = $ClaudeExe

function New-ClaudeCommand([string]$WindowsPathToken) {
  return "`"$WtExe`" -d `"$WindowsPathToken`" `"$ClaudeExe`""
}

$Targets = @(
  @{
    KeyPath = "HKLM:\Software\Classes\Directory\shell\$MenuName"
    Command = New-ClaudeCommand "%1"
  },
  @{
    KeyPath = "HKLM:\Software\Classes\Directory\Background\shell\$MenuName"
    Command = New-ClaudeCommand "%V"
  }
)

foreach ($t in $Targets) {
  $MenuKey = $t.KeyPath
  $CommandKey = Join-Path $MenuKey 'command'
  $Command = $t.Command

  New-Item -Path $MenuKey -Force | Out-Null
  New-Item -Path $CommandKey -Force | Out-Null

  New-ItemProperty -Path $MenuKey -Name '(Default)' -Value $MenuText -PropertyType String -Force | Out-Null
  New-ItemProperty -Path $MenuKey -Name 'Icon' -Value $IconPath -PropertyType String -Force | Out-Null

  New-ItemProperty -Path $CommandKey -Name '(Default)' -Value $Command -PropertyType String -Force | Out-Null
}

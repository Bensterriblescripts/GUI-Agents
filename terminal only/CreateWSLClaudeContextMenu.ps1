if (-not ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
  Start-Process powershell.exe -Verb RunAs -ArgumentList '-NoProfile','-ExecutionPolicy','Bypass','-File',('"{0}"'-f$PSCommandPath); exit
}

# Revert to ye old w10 context menus
reg.exe add "HKCU\Software\Classes\CLSID\{86ca1aa0-34aa-4e8b-a509-50c905bae2a2}\InprocServer32" /ve /d "" /f

$MenuName = 'Launch Claude WSL'
$MenuText = 'Launch Claude WSL'
$IconPath = "$env:WINDIR\System32\wsl.exe"
$WslExe   = "$env:WINDIR\System32\wsl.exe"

function New-WslClaudeCommand([string]$WindowsPathToken) {
  return "`"$WslExe`" --cd `"$WindowsPathToken`" -e bash -lc `"claude`""
}

$Targets = @(
  @{
    KeyPath  = "HKLM:\Software\Classes\Directory\shell\$MenuName"
    Command  = New-WslClaudeCommand "%1"   # clicked folder item (subfolder etc.)
  },
  @{
    KeyPath  = "HKLM:\Software\Classes\Directory\Background\shell\$MenuName"
    Command  = New-WslClaudeCommand "%V"   # current folder background
  }
)

foreach ($t in $Targets) {
  $MenuKey    = $t.KeyPath
  $CommandKey = Join-Path $MenuKey 'command'
  $Command    = $t.Command

  New-Item -Path $MenuKey -Force | Out-Null
  New-Item -Path $CommandKey -Force | Out-Null

  New-ItemProperty -Path $MenuKey -Name '(Default)' -Value $MenuText -PropertyType String -Force | Out-Null
  New-ItemProperty -Path $MenuKey -Name 'Icon'      -Value $IconPath -PropertyType String -Force | Out-Null

  New-ItemProperty -Path $CommandKey -Name '(Default)' -Value $Command -PropertyType String -Force | Out-Null
}

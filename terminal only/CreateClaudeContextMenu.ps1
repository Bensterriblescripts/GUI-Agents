if (-not ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
  Start-Process powershell.exe -Verb RunAs -ArgumentList '-NoProfile','-ExecutionPolicy','Bypass','-File',('"{0}"'-f$PSCommandPath); exit
}

reg.exe add "HKCU\Software\Classes\CLSID\{86ca1aa0-34aa-4e8b-a509-50c905bae2a2}\InprocServer32" /ve /d "" /f

$MenuName = 'Launch Claude'
$MenuText = 'Launch Claude'
$PreLaunchCommand = @'
$ClaudeHome = Join-Path $env:USERPROFILE '.claude'
@(
  'history.jsonl'
) | ForEach-Object {
  Remove-Item -LiteralPath (Join-Path $ClaudeHome $_) -Force -ErrorAction SilentlyContinue
}
@(
  'projects'
  'file-history'
  'backups'
) | ForEach-Object {
  Get-ChildItem -LiteralPath (Join-Path $ClaudeHome $_) -Force -ErrorAction SilentlyContinue | ForEach-Object {
    Remove-Item -LiteralPath $_.FullName -Recurse -Force -ErrorAction SilentlyContinue
  }
}
'@
$WtExe = (Get-Command wt.exe -ErrorAction Stop).Source
$ClaudeCommand = Get-Command claude -ErrorAction Stop | Select-Object -First 1
$ClaudeExeCommand = Get-Command claude.exe -ErrorAction SilentlyContinue | Select-Object -First 1
$PowerShellExe = (Get-Command powershell.exe -ErrorAction Stop).Source
$ClaudeLaunchTarget = $null
$ClaudeIcon = $null
$DefaultIcon = (Get-ItemProperty 'Registry::HKEY_CLASSES_ROOT\CLSID\{20D04FE0-3AEA-1069-A2D8-08002B30309D}\DefaultIcon' -Name '(default)' -ErrorAction SilentlyContinue).'(default)'

if ($ClaudeExeCommand -and $ClaudeExeCommand.Source) {
  $ClaudeIcon = $ClaudeExeCommand.Source
}

if ($ClaudeCommand.CommandType -eq 'Application' -and [IO.Path]::GetExtension($ClaudeCommand.Source) -ieq '.exe') {
  $ClaudeLaunchTarget = $ClaudeCommand.Source
  if (-not $ClaudeIcon) {
    $ClaudeIcon = $ClaudeCommand.Source
  }
} elseif ($ClaudeCommand.Source -and (Test-Path $ClaudeCommand.Source) -and [IO.Path]::GetExtension($ClaudeCommand.Source) -ieq '.exe') {
  $ClaudeLaunchTarget = $ClaudeCommand.Source
}

if (-not $ClaudeLaunchTarget -and $ClaudeExeCommand -and $ClaudeExeCommand.Source) {
  $ClaudeLaunchTarget = $ClaudeExeCommand.Source
}

$IconPath = if ($ClaudeIcon) { $ClaudeIcon } elseif ($DefaultIcon) { $DefaultIcon } else { "$env:SystemRoot\System32\imageres.dll,-109" }

function ConvertTo-SingleQuotedPowerShellString([string]$Value) {
  return "'" + ($Value -replace "'", "''") + "'"
}

function New-ClaudeCommand([string]$WindowsPathToken) {
  $ClaudeInvocation = if ($ClaudeLaunchTarget) {
    "& $(ConvertTo-SingleQuotedPowerShellString $ClaudeLaunchTarget) --dangerously-skip-permissions"
  } else {
    'claude --dangerously-skip-permissions'
  }
  $EncodedCommand = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes("Invoke-Expression $(ConvertTo-SingleQuotedPowerShellString $PreLaunchCommand); $ClaudeInvocation"))
  return "`"$WtExe`" -d `"$WindowsPathToken`" `"$PowerShellExe`" -NoExit -ExecutionPolicy Bypass -EncodedCommand $EncodedCommand"
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

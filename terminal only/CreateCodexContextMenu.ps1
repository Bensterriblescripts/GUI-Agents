if (-not ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
  Start-Process powershell.exe -Verb RunAs -ArgumentList '-NoProfile','-ExecutionPolicy','Bypass','-File',('"{0}"'-f$PSCommandPath); exit
}

reg.exe add "HKCU\Software\Classes\CLSID\{86ca1aa0-34aa-4e8b-a509-50c905bae2a2}\InprocServer32" /ve /d "" /f

$MenuName = 'Launch Codex'
$MenuText = 'Launch Codex'
$PreLaunchCommand = @'
$CodexHome = Join-Path $env:USERPROFILE '.codex'
@(
  'models_cache.json'
  'state_5.sqlite'
  'state_5.sqlite-shm'
  'state_5.sqlite-wal'
) | ForEach-Object {
  Remove-Item -LiteralPath (Join-Path $CodexHome $_) -Force -ErrorAction SilentlyContinue
}
Get-ChildItem -LiteralPath (Join-Path $CodexHome 'sessions') -Force -ErrorAction SilentlyContinue | ForEach-Object {
  Remove-Item -LiteralPath $_.FullName -Recurse -Force -ErrorAction SilentlyContinue
}
'@
$WtExe = (Get-Command wt.exe -ErrorAction Stop).Source
$CodexCommand = Get-Command codex -ErrorAction Stop | Select-Object -First 1
$CodexExeCommand = Get-Command codex.exe -ErrorAction SilentlyContinue | Select-Object -First 1
$PowerShellExe = (Get-Command powershell.exe -ErrorAction Stop).Source
$CodexLaunchTarget = $null
$CodexIcon = $null
$DefaultIcon = (Get-ItemProperty 'Registry::HKEY_CLASSES_ROOT\CLSID\{20D04FE0-3AEA-1069-A2D8-08002B30309D}\DefaultIcon' -Name '(default)' -ErrorAction SilentlyContinue).'(default)'

if ($CodexExeCommand -and $CodexExeCommand.Source) {
  $CodexIcon = $CodexExeCommand.Source
}

if ($CodexCommand.CommandType -eq 'Application' -and [IO.Path]::GetExtension($CodexCommand.Source) -ieq '.exe') {
  $CodexLaunchTarget = $CodexCommand.Source
  if (-not $CodexIcon) {
    $CodexIcon = $CodexCommand.Source
  }
} elseif ($CodexCommand.Source) {
  $ShimDirectory = Split-Path $CodexCommand.Source -Parent
  $BundledCodexExe = Join-Path $ShimDirectory 'node_modules\@openai\codex\node_modules\@openai\codex-win32-x64\vendor\x86_64-pc-windows-msvc\codex\codex.exe'
  if (Test-Path $BundledCodexExe) {
    $CodexLaunchTarget = $BundledCodexExe
  }
}

if (-not $CodexLaunchTarget -and $CodexExeCommand -and $CodexExeCommand.Source) {
  $CodexLaunchTarget = $CodexExeCommand.Source
}

$IconPath = if ($CodexIcon) { $CodexIcon } elseif ($DefaultIcon) { $DefaultIcon } else { "$env:SystemRoot\System32\imageres.dll,-109" }

function ConvertTo-SingleQuotedPowerShellString([string]$Value) {
  return "'" + ($Value -replace "'", "''") + "'"
}

function New-CodexCommand([string]$WindowsPathToken) {
  if (-not [string]::IsNullOrWhiteSpace($PreLaunchCommand)) {
    $CodexInvocation = if ($CodexLaunchTarget) {
      "& $(ConvertTo-SingleQuotedPowerShellString $CodexLaunchTarget)"
    } else {
      'codex'
    }
    $EncodedCommand = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes("Invoke-Expression $(ConvertTo-SingleQuotedPowerShellString $PreLaunchCommand); $CodexInvocation"))
    return "`"$WtExe`" -d `"$WindowsPathToken`" `"$PowerShellExe`" -NoExit -ExecutionPolicy Bypass -EncodedCommand $EncodedCommand"
  }

  if ($CodexLaunchTarget) {
    return "`"$WtExe`" -d `"$WindowsPathToken`" `"$CodexLaunchTarget`""
  }

  return "`"$WtExe`" -d `"$WindowsPathToken`" `"$PowerShellExe`" -NoExit -ExecutionPolicy Bypass -Command codex"
}

$Targets = @(
  @{
    KeyPath = "HKLM:\Software\Classes\Directory\shell\$MenuName"
    Command = New-CodexCommand "%1"
  },
  @{
    KeyPath = "HKLM:\Software\Classes\Directory\Background\shell\$MenuName"
    Command = New-CodexCommand "%V"
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

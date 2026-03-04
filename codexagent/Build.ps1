$ErrorActionPreference = 'Stop'

$repoRoot = $PSScriptRoot
$releaseDir = Join-Path $repoRoot 'target\release'
$sourceExe = Join-Path $releaseDir 'codexagent.exe'
$sourceDll = Join-Path $releaseDir 'codexagent_contextmenu.dll'
$targetDir = 'C:\Local\Software'
$targetExe = Join-Path $targetDir 'codexagent.exe'
$targetDll = Join-Path $targetDir 'codexagent_contextmenu.dll'

Push-Location $repoRoot
try {
  cargo build --release

  if (-not (Test-Path $sourceExe)) {
    throw "Build succeeded but $sourceExe was not found."
  }
  if (-not (Test-Path $sourceDll)) {
    throw "Build succeeded but $sourceDll was not found."
  }

  $running = Get-Process -Name 'codexagent' -ErrorAction SilentlyContinue | Where-Object {
    try {
      $_.Path -eq $targetExe
    }
    catch {
      $false
    }
  }

  if ($running) {
      Write-Output "Error: Codex is still running. The executable has not been updated."
      return
  }

  New-Item -ItemType Directory -Path $targetDir -Force | Out-Null
  Copy-Item -Path $sourceExe -Destination $targetExe -Force
  Copy-Item -Path $sourceDll -Destination $targetDll -Force
}
finally {
  Pop-Location
}

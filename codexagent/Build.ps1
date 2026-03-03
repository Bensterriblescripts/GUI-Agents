$ErrorActionPreference = 'Stop'

$repoRoot = $PSScriptRoot
$releaseDir = Join-Path $repoRoot 'target\release'
$sourceExe = Join-Path $releaseDir 'codexagent.exe'
$targetDir = 'C:\Local\Software'
$targetExe = Join-Path $targetDir 'codexagent.exe'

Push-Location $repoRoot
try {
  cargo build --release

  if (-not (Test-Path $sourceExe)) {
    throw "Build succeeded but $sourceExe was not found."
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
      Write-Output "A Codex agent is still running, the executable has been built but not updated"
      return
  }

  New-Item -ItemType Directory -Path $targetDir -Force | Out-Null
  Copy-Item -Path $sourceExe -Destination $targetExe -Force
}
finally {
  Pop-Location
}

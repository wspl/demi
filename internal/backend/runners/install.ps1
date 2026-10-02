#requires -Version 5.1
$ErrorActionPreference = 'Stop'
$demiBackend = @BACKEND@
$demiBase = @BASE@
$demiRelease = @RELEASE@
$demiRegistration = @REGISTRATION@
$demiArtifacts = @{
@ARTIFACTS@
}
switch ([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()) {
  'Arm64' { $demiTarget = 'aarch64-pc-windows-msvc' }
  'X64' { $demiTarget = 'x86_64-pc-windows-msvc' }
  default { throw 'Unsupported runner architecture' }
}
$demiInstance = if ($env:DEMI_INSTALLATION_ID) { $env:DEMI_INSTALLATION_ID } else { $demiRegistration }
if ($demiInstance -notmatch '^[a-zA-Z0-9_-]+$') { throw 'Invalid installation ID' }
$demiHome = if ($env:USERPROFILE) { $env:USERPROFILE } else { [Environment]::GetFolderPath('UserProfile') }
$demiState = Join-Path $demiHome ".demi/instances/$demiInstance"
$demiReleases = Join-Path $demiState 'releases'
$demiBin = Join-Path $demiReleases $demiRelease
$demiExe = Join-Path $demiBin 'demi-runner.exe'
[IO.Directory]::CreateDirectory($demiReleases) | Out-Null
$demiBackendFile = Join-Path $demiState 'backend-url'
$demiLockFile = Join-Path $demiState 'install.lock'
$demiLock = [IO.File]::Open($demiLockFile, [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::Write, [IO.FileShare]::None)
$demiStage = Join-Path $demiReleases ('.download-' + [Guid]::NewGuid().ToString('N'))
$demiPreviousHome = $env:DEMI_HOME
$demiPreviousRelease = $env:DEMI_RELEASE_ID
$demiProcess = $null
function Invoke-DemiControl([string]$Action) {
  $control = [Diagnostics.ProcessStartInfo]::new()
  $control.FileName = $demiExe
  $control.Arguments = $Action + ' --backend "' + $demiBackend + '"'
  $control.UseShellExecute = $false
  $control.CreateNoWindow = $true
  $control.RedirectStandardOutput = $true
  $control.RedirectStandardError = $true
  $process = [Diagnostics.Process]::Start($control)
  try {
    $output = $process.StandardOutput.ReadToEndAsync()
    $errors = $process.StandardError.ReadToEndAsync()
    $process.WaitForExit()
    $output.GetAwaiter().GetResult() | Out-Null
    $errors.GetAwaiter().GetResult() | Out-Null
    return $process.ExitCode
  } finally {
    $process.Dispose()
  }
}
try {
  if ((Test-Path $demiBackendFile) -and ([IO.File]::ReadAllText($demiBackendFile).Trim() -ne $demiBackend)) {
    throw 'Installation belongs to another backend'
  }
  [IO.Directory]::CreateDirectory($demiStage) | Out-Null
  $demiDownload = Join-Path $demiStage 'demi-runner.exe'
  Write-Output "Downloading runner for $demiTarget..."
  $demiPreviousProgress = $ProgressPreference
  try {
    $ProgressPreference = 'SilentlyContinue'
    Invoke-WebRequest -UseBasicParsing -Uri "$demiBase/runner-artifacts/$demiRelease/$demiTarget/demi-runner.exe" -OutFile $demiDownload
  } finally {
    $ProgressPreference = $demiPreviousProgress
  }
  if ((Get-Item $demiDownload).Length -ne $demiArtifacts[$demiTarget].Size -or
      (Get-FileHash -Algorithm SHA256 $demiDownload).Hash.ToLowerInvariant() -ne $demiArtifacts[$demiTarget].Hash) {
    throw 'Runner download checksum mismatch'
  }
  if (Test-Path $demiBin) {
    if ((Get-Item $demiExe).Length -ne $demiArtifacts[$demiTarget].Size -or
        (Get-FileHash -Algorithm SHA256 $demiExe).Hash.ToLowerInvariant() -ne $demiArtifacts[$demiTarget].Hash) {
      throw 'Installed runner checksum mismatch'
    }
  } else {
    Move-Item $demiStage $demiBin
  }
  $env:DEMI_HOME = $demiState
  $env:DEMI_RELEASE_ID = $demiRelease
  $demiStatus = Invoke-DemiControl 'status'
  if ($demiStatus -eq 0) {
    Write-Output "Runner already running: $demiState"
    return
  }
  if ($demiStatus -eq 3) {
    Write-Output 'Waiting for existing jobs before upgrading this runner...'
    if ((Invoke-DemiControl 'drain') -ne 0) { throw 'Runner drain failed' }
  }
  $demiUtf8 = [Text.UTF8Encoding]::new($false)
  [IO.File]::WriteAllText($demiBackendFile, $demiBackend + [Environment]::NewLine, $demiUtf8)
  [IO.File]::WriteAllText((Join-Path $demiState 'release-id'), $demiRelease + [Environment]::NewLine, $demiUtf8)
  $demiLauncher = @'
param([ValidateSet('run', 'status', 'drain')][string]$Action = 'run')
$ErrorActionPreference = 'Stop'
$demiState = $PSScriptRoot
$demiBackend = [IO.File]::ReadAllText((Join-Path $demiState 'backend-url')).Trim()
$demiRelease = [IO.File]::ReadAllText((Join-Path $demiState 'release-id')).Trim()
$demiPreviousHome = $env:DEMI_HOME
$demiPreviousRelease = $env:DEMI_RELEASE_ID
try {
  $env:DEMI_HOME = $demiState
  $env:DEMI_RELEASE_ID = $demiRelease
  & (Join-Path $demiState "releases/$demiRelease/demi-runner.exe") $Action --backend $demiBackend
  $demiExit = $LASTEXITCODE
} finally {
  $env:DEMI_HOME = $demiPreviousHome
  $env:DEMI_RELEASE_ID = $demiPreviousRelease
}
exit $demiExit
'@
  [IO.File]::WriteAllText((Join-Path $demiState 'run.ps1'), $demiLauncher, $demiUtf8)
  Write-Output 'Starting runner...'
  $demiProcess = Start-Process -FilePath $demiExe -ArgumentList @('run', '--backend', $demiBackend) -WorkingDirectory $demiHome -WindowStyle Hidden -RedirectStandardOutput (Join-Path $demiState 'runner.stdout.log') -RedirectStandardError (Join-Path $demiState 'runner.log') -PassThru
  $demiStarted = $false
  for ($demiAttempt = 0; $demiAttempt -lt 100; $demiAttempt++) {
    if ((Invoke-DemiControl 'status') -eq 0) {
      $demiStarted = $true
      break
    }
    if ($demiProcess.HasExited) {
      break
    }
    Start-Sleep -Milliseconds 100
  }
  if (-not $demiStarted) { throw ([IO.File]::ReadAllText((Join-Path $demiState 'runner.log'))) }
  Write-Output "Runner installed for $demiBackend"
  Write-Output "State and pairing log: $demiState/runner.log"
} catch {
  if ($demiProcess -and -not $demiProcess.HasExited) {
    $demiProcess.Kill()
    $demiProcess.WaitForExit()
  }
  throw
} finally {
  $env:DEMI_HOME = $demiPreviousHome
  $env:DEMI_RELEASE_ID = $demiPreviousRelease
  $demiLock.Dispose()
  if ($demiProcess) { $demiProcess.Dispose() }
  if (Test-Path $demiStage) { Remove-Item -Recurse -Force $demiStage }
}

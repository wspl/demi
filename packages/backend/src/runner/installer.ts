import { createHash } from 'node:crypto'
import type { RunnerRelease } from '@demicodes/runner-protocol/release'

function quote(value: string): string {
  return `'${value.replaceAll("'", "'\\''")}'`
}

export function shellInstaller(
  backend: string,
  release: RunnerRelease
): string {
  const url = new URL(backend)
  const http = url.protocol === 'https:' || url.protocol === 'http:'
  if (!http || url.username || url.password || url.search || url.hash) {
    throw new Error('invalid installation backend URL')
  }
  const registration = createHash('sha256').update(url.toString()).digest('hex')
  const cases = Object.entries(release.targets).map(([name, hashes]) => `
  ${name})
    runner_hash=${quote(hashes.sha256)}
    ;;`).join('\n')
  return `#!/bin/sh
set -eu
backend=${quote(url.toString())}
release=${quote(release.release)}
registration=${quote(registration)}
base=${quote(url.origin)}
case "$(uname -s):$(uname -m)" in
  Darwin:arm64) target=aarch64-apple-darwin ;;
  Darwin:x86_64) target=x86_64-apple-darwin ;;
  Linux:aarch64|Linux:arm64) target=aarch64-unknown-linux-musl ;;
  Linux:x86_64) target=x86_64-unknown-linux-musl ;;
  *)
    echo 'Unsupported runner platform' >&2
    exit 1
    ;;
esac
case "$target" in
${cases}
  *)
    echo 'This backend has no runner release for this platform' >&2
    exit 1
    ;;
esac
instance=\${DEMI_INSTALLATION_ID:-$registration}
case "$instance" in
  ''|*[!a-zA-Z0-9_-]*)
    echo 'Invalid installation ID' >&2
    exit 1
    ;;
esac
state="$HOME/.demi/instances/$instance"
bin="$state/releases/$release"
umask 077
mkdir -p "$state/releases"
if [ -f "$state/backend-url" ]; then
  IFS= read -r existing_backend < "$state/backend-url"
  if [ "$existing_backend" != "$backend" ]; then
    echo 'Installation belongs to another backend' >&2
    exit 1
  fi
fi
if ! mkdir "$state/install.lock" 2>/dev/null; then
  echo 'Another installer is active for this installation' >&2
  exit 1
fi
stage=
cleanup() {
  if [ -n "$stage" ]; then
    rm -r "$stage"
  fi
  rmdir "$state/install.lock"
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM
stage=$(mktemp -d "$state/releases/.download-XXXXXX")
verify() {
  if command -v sha256sum >/dev/null 2>&1; then
    actual=$(sha256sum "$1")
  else
    actual=$(shasum -a 256 "$1")
  fi
  actual=\${actual%% *}
  if [ "$actual" != "$2" ]; then
    echo 'Runner download checksum mismatch' >&2
    exit 1
  fi
}
curl -fSL "$base/runner-artifacts/$release/$target/demi-runner" -o "$stage/demi-runner"
verify "$stage/demi-runner" "$runner_hash"
chmod 755 "$stage/demi-runner"
if [ -d "$bin" ]; then
  verify "$bin/demi-runner" "$runner_hash"
else
  mv "$stage" "$bin"
  mkdir "$stage"
fi
if DEMI_HOME="$state" DEMI_RELEASE_ID="$release" "$bin/demi-runner" status --backend "$backend" >/dev/null 2>&1; then
  echo "Runner already running: $state"
  exit 0
else
  status=$?
  if [ "$status" -eq 3 ]; then
    echo 'Waiting for existing jobs before upgrading this runner…'
    DEMI_HOME="$state" "$bin/demi-runner" drain --backend "$backend"
  fi
fi
# Pass values as quoted arguments; never interpolate a backend into executable shell code.
cat > "$state/run.next" <<'LAUNCHER'
#!/bin/sh
set -eu
state=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
IFS= read -r backend < "$state/backend-url"
IFS= read -r release < "$state/release-id"
export DEMI_HOME="$state" DEMI_RELEASE_ID="$release"
exec "$state/releases/$release/demi-runner" "\${1:-run}" --backend "$backend"
LAUNCHER
printf '%s\\n' "$backend" > "$state/backend-url"
printf '%s\\n' "$release" > "$state/release-id"
chmod 755 "$state/run.next"
mv "$state/run.next" "$state/run"
nohup "$state/run" > "$state/runner.log" 2>&1 < /dev/null &
pid=$!
tries=0
until "$state/run" status >/dev/null 2>&1; do
  tries=$((tries + 1))
  if ! kill -0 "$pid" 2>/dev/null || [ "$tries" -ge 100 ]; then
    cat "$state/runner.log" >&2
    exit 1
  fi
  sleep 0.1
done
printf 'Runner installed for %s\\nState and pairing log: %s\\n' "$backend" "$state/runner.log"
cat "$state/runner.log"
`
}

function powershellQuote(value: string): string {
  return `'${value.replaceAll("'", "''")}'`
}

/** Installs the same native runner on Windows without a separate client binary. */
export function powershellInstaller(backend: string, release: RunnerRelease): string {
  const url = new URL(backend)
  if (!['https:', 'http:'].includes(url.protocol) || url.username || url.password || url.search || url.hash)
    throw new Error('invalid installation backend URL')
  const registration = createHash('sha256').update(url.toString()).digest('hex')
  const artifacts = Object.entries(release.targets).filter(([target]) => target.includes('windows'))
    .map(([target, artifact]) => `  ${powershellQuote(target)} = @{ Hash = ${powershellQuote(artifact.sha256)}; Size = ${artifact.size} }`).join('\n')
  return `#requires -Version 5.1
$ErrorActionPreference = 'Stop'
$demiBackend = ${powershellQuote(url.toString())}
$demiBase = ${powershellQuote(url.origin)}
$demiRelease = ${powershellQuote(release.release)}
$demiRegistration = ${powershellQuote(registration)}
$demiArtifacts = @{
${artifacts}
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
  Invoke-WebRequest -UseBasicParsing -Uri "$demiBase/runner-artifacts/$demiRelease/$demiTarget/demi-runner.exe" -OutFile $demiDownload
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
`
}

//! The runner installers (`builds-and-releases.md` § Packaging, `web-api.md`
//! § Workspaces, devices, and attached hosts): a shell script for macOS and
//! Linux and a PowerShell script for Windows, each naming the backend and
//! one runner release. A script downloads its platform's runner from the
//! backend, checks its SHA-256, installs it under an installation of its own
//! per backend, drains an older runner of that installation before it
//! upgrades, and starts the runner, which asks to be paired. It then shows
//! the runner's pairing codes until the runner is paired, and the device's
//! name and the removal command once it is (`runner.md` § Installation,
//! pairing and removal). A script holds no credential: pairing grants device
//! access.

use std::path::Path;

use demi_runner_protocol::console::{PAIRED, PAIRING_CODE, REMOVAL};
use demi_runner_protocol::release::RunnerRelease;
use demi_runner_protocol::wire::RunnerPlatform;
use sha2::{Digest, Sha256};
use url::Url;

/// A backend URL an installer cannot name: not HTTP or HTTPS, or with a
/// user, a password, a query or a fragment.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0} cannot be an installation's backend URL")]
pub struct InvalidBackendUrl(String);

/// `url` as an installer names it.
pub fn backend_url(url: &Url) -> Result<Url, InvalidBackendUrl> {
    let usable = matches!(url.scheme(), "http" | "https")
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none();
    if usable {
        Ok(url.clone())
    } else {
        Err(InvalidBackendUrl(url.to_string()))
    }
}

/// The installation a backend's scripts install into unless the user names
/// another: the SHA-256 of its URL, so that each backend's runner has its
/// own.
fn registration(backend: &Url) -> String {
    hex::encode(Sha256::digest(backend.as_str().as_bytes()))
}

/// The command that starts the runner of `backend`'s installation again on
/// a device of `platform`, typed in a terminal there: the launcher of the
/// installation, in `installation` as its runner reported it or where the
/// installers put it by default, with `start`, which starts the runner in
/// the background (`runner.md` § Installation, pairing and removal). A
/// directory in `home`, the device's home directory, is written from the
/// home: `~` in a POSIX shell, `$env:USERPROFILE` in PowerShell, where `-File`
/// does not expand `~`. On Windows it runs through PowerShell with a policy
/// that lets the launcher run.
pub fn start_command(
    backend: &Url,
    installation: Option<&str>,
    home: Option<&str>,
    platform: RunnerPlatform,
) -> String {
    let windows = platform == RunnerPlatform::Win32;
    let separator = if windows { '\\' } else { '/' };
    let default = format!(".demi{separator}instances{separator}{}", registration(backend));
    let place = match installation {
        None => Place::Home(default),
        Some(installation) => {
            let relative = home
                .and_then(|home| installation.strip_prefix(home.trim_end_matches(separator)))
                .and_then(|rest| rest.strip_prefix(separator));
            match relative {
                Some(relative) => Place::Home(relative.to_owned()),
                None => Place::Absolute(installation.to_owned()),
            }
        }
    };
    if windows {
        let launcher = match place {
            Place::Home(relative) => format!(
                "\"$env:USERPROFILE\\{}\\run.ps1\"",
                powershell_expandable(&relative)
            ),
            Place::Absolute(installation) => powershell(&format!("{installation}\\run.ps1")),
        };
        return format!("powershell -ExecutionPolicy Bypass -File {launcher} start");
    }
    match place {
        Place::Home(relative) => format!("~/{} start", sh(&format!("{relative}/run"))),
        Place::Absolute(installation) => format!("{} start", sh(&format!("{installation}/run"))),
    }
}

/// Where an installation's directory is, as a command names it.
enum Place {
    /// Under the home directory, at this path relative to it.
    Home(String),
    Absolute(String),
}

/// `value` inside a PowerShell string literal that expands variables, with
/// nothing of its own expanded.
fn powershell_expandable(value: &str) -> String {
    value
        .replace('`', "``")
        .replace('"', "`\"")
        .replace('$', "`$")
}

/// A server release's `runners/`, where the runner releases are
/// (`native-runtime.md` § Runner releases): `manifest.json` names the
/// current one, the release the installers install and every paired
/// device's runner follows, and each release's directory holds its own,
/// which names each target's executable. A backend without them, as in
/// development, serves no installer and checks no runner's release.
#[derive(Debug, Clone, Default)]
pub struct RunnerReleases(Option<std::path::PathBuf>);

impl RunnerReleases {
    pub fn new(root: Option<std::path::PathBuf>) -> Self {
        Self(root)
    }

    /// The directory, when the backend has runner releases.
    pub fn root(&self) -> Option<&Path> {
        self.0.as_deref()
    }

    /// The current runner release; none without runner releases. The
    /// record is read on each use, so a release packaged into a running
    /// backend's `runners/` takes effect at once.
    pub async fn current(&self) -> Result<Option<RunnerRelease>, String> {
        let Some(root) = &self.0 else {
            return Ok(None);
        };
        let path = root.join("manifest.json");
        match read_runner_release(&path).await? {
            Some(release) => Ok(Some(release)),
            None => Err(format!("{} is missing", path.display())),
        }
    }
}

/// The runner release record at `path`, a release's or the top-level
/// `manifest.json` of a server release's `runners/`, when there is one; a
/// record that does not decode is the deployment's fault.
pub async fn read_runner_release(path: &Path) -> Result<Option<RunnerRelease>, String> {
    let bytes = match tokio::fs::read(path).await {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("{}: {error}", path.display())),
    };
    RunnerRelease::decode(&bytes)
        .map(Some)
        .map_err(|error| format!("{}: {error}", path.display()))
}

/// `value` as one POSIX shell word.
fn sh(value: &str) -> String {
    shlex::try_quote(value)
        .expect("an installer's values hold no NUL byte")
        .into_owned()
}

/// `value` as a PowerShell string literal.
fn powershell(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// The shell installer of `release` for `backend`.
pub fn shell_script(backend: &Url, release: &RunnerRelease) -> String {
    let cases: Vec<String> = release
        .targets
        .iter()
        .map(|(target, artifact)| {
            format!(
                "\n  {target})\n    runner_hash={}\n    ;;",
                sh(&artifact.sha256)
            )
        })
        .collect();
    SHELL_SCRIPT
        .replace("@BACKEND@", &sh(backend.as_str()))
        .replace("@RELEASE@", &sh(&release.release))
        .replace("@REGISTRATION@", &sh(&registration(backend)))
        .replace("@BASE@", &sh(&backend.origin().ascii_serialization()))
        .replace("@PAIRING@", &sh(PAIRING_CODE))
        .replace("@PAIRED@", &sh(PAIRED))
        .replace("@REMOVAL@", &sh(REMOVAL))
        .replace("@CASES@", &cases.join("\n"))
}

/// The PowerShell installer of `release` for `backend`, for the release's
/// Windows targets.
pub fn powershell_script(backend: &Url, release: &RunnerRelease) -> String {
    let artifacts: Vec<String> = release
        .targets
        .iter()
        .filter(|(target, _)| target.contains("windows"))
        .map(|(target, artifact)| {
            format!(
                "  {} = @{{ Hash = {}; Size = {} }}",
                powershell(target),
                powershell(&artifact.sha256),
                artifact.size
            )
        })
        .collect();
    POWERSHELL_SCRIPT
        .replace("@BACKEND@", &powershell(backend.as_str()))
        .replace(
            "@BASE@",
            &powershell(&backend.origin().ascii_serialization()),
        )
        .replace("@RELEASE@", &powershell(&release.release))
        .replace("@REGISTRATION@", &powershell(&registration(backend)))
        .replace("@PAIRING@", &powershell(PAIRING_CODE))
        .replace("@PAIRED@", &powershell(PAIRED))
        .replace("@REMOVAL@", &powershell(REMOVAL))
        .replace("@ARTIFACTS@", &artifacts.join("\n"))
}

const SHELL_SCRIPT: &str = r#"#!/bin/sh
set -eu
backend=@BACKEND@
release=@RELEASE@
registration=@REGISTRATION@
base=@BASE@
pairing_prefix=@PAIRING@
paired_prefix=@PAIRED@
removal_prefix=@REMOVAL@
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
@CASES@
  *)
    echo 'This backend has no runner release for this platform' >&2
    exit 1
    ;;
esac
instance=${DEMI_INSTALLATION_ID:-$registration}
case "$instance" in
  ''|*[!a-zA-Z0-9_-]*)
    echo 'Invalid installation ID' >&2
    exit 1
    ;;
esac
state="$HOME/.demi/instances/$instance"
bin="$state/releases/$release"
# The installation's files are the user's alone; the runner works with the
# mask of the shell this installer runs in (runner.md § Builtins that act on
# a process).
user_umask=$(umask)
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
  actual=${actual%% *}
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
pid=
if DEMI_HOME="$state" DEMI_RELEASE_ID="$release" "$bin/demi-runner" status --backend "$backend" >/dev/null 2>&1; then
  echo "Runner already running: $state"
else
  status=$?
  if [ "$status" -eq 3 ]; then
    echo 'Waiting for existing jobs before upgrading this runner…'
    DEMI_HOME="$state" "$bin/demi-runner" drain --backend "$backend"
  fi
  # Pass values as quoted arguments; never interpolate a backend into executable shell code.
  cat > "$state/run.next" <<'LAUNCHER'
#!/bin/sh
set -eu
state=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
IFS= read -r backend < "$state/backend-url"
IFS= read -r release < "$state/release-id"
export DEMI_HOME="$state" DEMI_RELEASE_ID="$release"
exec "$state/releases/$release/demi-runner" "${1:-run}" --backend "$backend"
LAUNCHER
  printf '%s\n' "$backend" > "$state/backend-url"
  printf '%s\n' "$release" > "$state/release-id"
  chmod 755 "$state/run.next"
  mv "$state/run.next" "$state/run"
  # The log is made here, under 077: it holds the pairing code. With job
  # control on, the runner starts in a process group of its own, so that
  # interrupting this installer at its terminal leaves the runner running.
  set -m 2>/dev/null
  nohup sh -c 'umask "$1" && exec "$2"' sh "$user_umask" "$state/run" > "$state/runner.log" 2>&1 < /dev/null &
  pid=$!
  set +m
  printf 'Runner installed for %s\n' "$backend"
fi
# The installation is in place; the runner's own updates may take its lock
# while this installer waits for pairing.
trap - EXIT
cleanup
# Shows the runner's log as pairing goes: each pairing code the runner
# receives, then the device's name and the removal command once it is
# paired. Only complete lines are read; of the codes read at once, only the
# last is still live.
log="$state/runner.log"
seen=0
shown=
while :; do
  lines=$(($(wc -l < "$log")))
  code=
  name=
  removal=
  if [ "$lines" -gt "$seen" ]; then
    chunk=$(sed -n "$((seen + 1)),${lines}p" "$log")
    seen=$lines
    while IFS= read -r line; do
      case "$line" in
        "$pairing_prefix"*) code=${line#"$pairing_prefix"} ;;
        "$paired_prefix"*) name=${line#"$paired_prefix"} ;;
        "$removal_prefix"*) removal=${line#"$removal_prefix"} ;;
      esac
    done <<CHUNK
$chunk
CHUNK
  fi
  if [ -n "$removal" ]; then
    printf 'Paired as %s\nTo remove this runner, run: %s\n' "$name" "$removal"
    exit 0
  fi
  if [ -n "$code" ] && [ "$code" != "$shown" ]; then
    if [ -z "$shown" ]; then
      printf 'Enter this pairing code in Add Device: %s\n' "$code"
    else
      printf 'The code expired; enter this one instead: %s\n' "$code"
    fi
    shown=$code
  fi
  if [ -n "$pid" ]; then
    alive=$(kill -0 "$pid" 2>/dev/null && echo yes || echo no)
  else
    alive=$("$state/run" status >/dev/null 2>&1 && echo yes || echo no)
  fi
  if [ "$alive" = no ]; then
    echo 'The runner stopped:' >&2
    cat "$log" >&2
    exit 1
  fi
  sleep 0.2
done
"#;

const POWERSHELL_SCRIPT: &str = r#"#requires -Version 5.1
$ErrorActionPreference = 'Stop'
$demiBackend = @BACKEND@
$demiBase = @BASE@
$demiRelease = @RELEASE@
$demiRegistration = @REGISTRATION@
$demiPairingPrefix = @PAIRING@
$demiPairedPrefix = @PAIRED@
$demiRemovalPrefix = @REMOVAL@
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
    } else {
      if ($demiStatus -eq 3) {
        Write-Output 'Waiting for existing jobs before upgrading this runner...'
        if ((Invoke-DemiControl 'drain') -ne 0) { throw 'Runner drain failed' }
      }
      $demiUtf8 = [Text.UTF8Encoding]::new($false)
      [IO.File]::WriteAllText($demiBackendFile, $demiBackend + [Environment]::NewLine, $demiUtf8)
      [IO.File]::WriteAllText((Join-Path $demiState 'release-id'), $demiRelease + [Environment]::NewLine, $demiUtf8)
      $demiLauncher = @'
param([ValidateSet('run', 'start', 'status', 'drain', 'uninstall')][string]$Action = 'run')
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
      Write-Output "Runner installed for $demiBackend"
    }
  } catch {
    if ($demiProcess -and -not $demiProcess.HasExited) {
      $demiProcess.Kill()
      $demiProcess.WaitForExit()
    }
    throw
  } finally {
    # The installation is in place; the runner's own updates may take its
    # lock while this installer waits for pairing.
    $demiLock.Dispose()
    if (Test-Path $demiStage) { Remove-Item -Recurse -Force $demiStage }
  }
  # Shows the runner's log as pairing goes: each pairing code the runner
  # receives, then the device's name and the removal command once it is
  # paired. Of the codes read at once, only the last is still live.
  # Interrupting this installer leaves the runner running.
  $demiLog = Join-Path $demiState 'runner.log'
  $demiReader = [IO.StreamReader]::new([IO.FileStream]::new($demiLog, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]'ReadWrite, Delete'))
  try {
    $demiShown = $null
    while ($true) {
      $demiCode = $null
      $demiName = $null
      $demiRemoval = $null
      while ($null -ne ($demiLine = $demiReader.ReadLine())) {
        if ($demiLine.StartsWith($demiPairingPrefix)) {
          $demiCode = $demiLine.Substring($demiPairingPrefix.Length)
        } elseif ($demiLine.StartsWith($demiPairedPrefix)) {
          $demiName = $demiLine.Substring($demiPairedPrefix.Length)
        } elseif ($demiLine.StartsWith($demiRemovalPrefix)) {
          $demiRemoval = $demiLine.Substring($demiRemovalPrefix.Length)
        }
      }
      if ($demiRemoval) {
        Write-Output "Paired as $demiName"
        Write-Output "To remove this runner, run: $demiRemoval"
        break
      }
      if ($demiCode -and $demiCode -ne $demiShown) {
        if ($demiShown) {
          Write-Output "The code expired; enter this one instead: $demiCode"
        } else {
          Write-Output "Enter this pairing code in Add Device: $demiCode"
        }
        $demiShown = $demiCode
      }
      $demiAlive = if ($demiProcess) { -not $demiProcess.HasExited } else { (Invoke-DemiControl 'status') -eq 0 }
      if (-not $demiAlive) {
        throw ('The runner stopped:' + [Environment]::NewLine + [IO.File]::ReadAllText($demiLog))
      }
      Start-Sleep -Milliseconds 200
    }
  } finally {
    $demiReader.Dispose()
  }
} finally {
  $env:DEMI_HOME = $demiPreviousHome
  $env:DEMI_RELEASE_ID = $demiPreviousRelease
  if ($demiProcess) { $demiProcess.Dispose() }
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    /// The start command names the installation the runner reported,
    /// written from the home when it lies there, or the default one.
    #[test]
    fn the_start_command_names_the_reported_installation_from_the_home() {
        let backend = Url::parse("https://demi.example.com/").unwrap();
        let id = registration(&backend);
        let cases: [(Option<&str>, Option<&str>, RunnerPlatform, String); 6] = [
            (None, None, RunnerPlatform::Darwin, format!("~/.demi/instances/{id}/run start")),
            (
                Some("/Users/ana/.demi/instances/work"),
                Some("/Users/ana"),
                RunnerPlatform::Darwin,
                "~/.demi/instances/work/run start".into(),
            ),
            (
                Some("/Users/ana/my demi"),
                Some("/Users/ana/"),
                RunnerPlatform::Linux,
                "~/'my demi/run' start".into(),
            ),
            (
                Some("/opt/demi"),
                Some("/Users/ana"),
                RunnerPlatform::Linux,
                "/opt/demi/run start".into(),
            ),
            (
                Some("C:\\Users\\ana\\.demi\\instances\\work"),
                Some("C:\\Users\\ana"),
                RunnerPlatform::Win32,
                "powershell -ExecutionPolicy Bypass -File \"$env:USERPROFILE\\.demi\\instances\\work\\run.ps1\" start".into(),
            ),
            (
                Some("D:\\demi's"),
                Some("C:\\Users\\ana"),
                RunnerPlatform::Win32,
                "powershell -ExecutionPolicy Bypass -File 'D:\\demi''s\\run.ps1' start".into(),
            ),
        ];
        for (installation, home, platform, expected) in cases {
            assert_eq!(start_command(&backend, installation, home, platform), expected);
        }
    }

    #[test]
    fn a_backend_url_names_an_http_origin_and_nothing_to_leak() {
        for usable in ["http://localhost:3271/", "https://demi.example.com/base"] {
            assert!(
                backend_url(&Url::parse(usable).unwrap()).is_ok(),
                "{usable}"
            );
        }
        for refused in [
            "ftp://demi.example.com/",
            "https://user@demi.example.com/",
            "https://user:secret@demi.example.com/",
            "https://demi.example.com/?token=1",
            "https://demi.example.com/#fragment",
        ] {
            assert!(
                backend_url(&Url::parse(refused).unwrap()).is_err(),
                "{refused}"
            );
        }
    }
}

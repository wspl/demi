//! The name of a runner installation's directory, which the installers and the
//! runner both derive from the backend's URL, so that no two of them can
//! disagree about it: `~/.demi/runner/<name>`.
//!
//! The name is the backend's host, then its port when it is not the scheme's
//! default, then the URL's path segments, joined by `-`, and last the first
//! four hex digits of the SHA-256 of the whole URL: `demi.example.com-3f9a`,
//! `127.0.0.1-3310-b27e`. A person reads from it which backend an installation
//! serves, and the hash keeps apart the backends the readable part cannot, such
//! as `http://example.com` and `https://example.com`, or `/team-a` and
//! `/team/a`. The URL is hashed as it is written, so `/Team` and `/team`, which
//! a case-insensitive file system would otherwise read as one directory, are
//! two names.
//!
//! The name is a safe file name on every system. In the readable part only
//! ASCII letters and digits, `.`, `-` and `_` stay; every other character, such
//! as the `:` of an IPv6 host or a percent-encoded byte's `%`, becomes `_`. The
//! hash also ends the name, so no name is a device name Windows reserves, and
//! none ends in a dot.
//!
//! Two backends whose names agree up to the hash would share a directory, one
//! chance in 65,536 when their readable parts agree.

use sha2::{Digest, Sha256};
use url::{Host, Url};

/// The launcher an installer writes as `run` in an installation, which its
/// service registration runs: it starts the installation's runner, the file
/// [`RUNNER`](crate::release::RUNNER) in its directory (`.exe` on Windows),
/// with the backend and the release the installation records. Every release
/// runs from that one file, so the system tells one program across updates,
/// as a Mac's privacy permissions need: they follow a program's path. The
/// runner writes the launcher too, when it brings an installation from before
/// to this layout (`crates/runner/src/update.rs`).
pub const LAUNCHER_SH: &str = r#"#!/bin/sh
set -eu
state=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
IFS= read -r backend < "$state/backend-url"
IFS= read -r release < "$state/release-id"
export DEMI_HOME="$state" DEMI_RELEASE_ID="$release"
action=${1:-run}
[ "$#" -eq 0 ] || shift
exec "$state/demi-runner" "$action" --backend "$backend" "$@"
"#;

/// The launcher an installer writes as `run.ps1` on Windows, as `LAUNCHER_SH`.
pub const LAUNCHER_PS1: &str = r#"param(
  [ValidateSet('run', 'start', 'status', 'drain', 'uninstall')][string]$Action = 'run',
  [Parameter(ValueFromRemainingArguments)][string[]]$Rest = @()
)
$ErrorActionPreference = 'Stop'
$demiState = $PSScriptRoot
$demiBackend = [IO.File]::ReadAllText((Join-Path $demiState 'backend-url')).Trim()
$demiRelease = [IO.File]::ReadAllText((Join-Path $demiState 'release-id')).Trim()
$demiPreviousHome = $env:DEMI_HOME
$demiPreviousRelease = $env:DEMI_RELEASE_ID
try {
  $env:DEMI_HOME = $demiState
  $env:DEMI_RELEASE_ID = $demiRelease
  & (Join-Path $demiState 'demi-runner.exe') $Action --backend $demiBackend @Rest
  $demiExit = $LASTEXITCODE
} finally {
  $env:DEMI_HOME = $demiPreviousHome
  $env:DEMI_RELEASE_ID = $demiPreviousRelease
}
exit $demiExit
"#;

/// The directories under the user's home that hold every installation.
pub const ROOT: [&str; 2] = [".demi", "runner"];

/// The directory name of the installation of `backend`.
pub fn installation_name(backend: &Url) -> String {
    let mut parts: Vec<String> = Vec::new();
    match backend.host() {
        Some(Host::Domain(domain)) => parts.push(domain.to_owned()),
        Some(Host::Ipv4(address)) => parts.push(address.to_string()),
        Some(Host::Ipv6(address)) => parts.push(address.to_string()),
        None => {}
    }
    if let Some(port) = backend.port() {
        parts.push(port.to_string());
    }
    if let Some(segments) = backend.path_segments() {
        parts.extend(segments.filter(|segment| !segment.is_empty()).map(str::to_owned));
    }
    let readable: String = parts
        .join("-")
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect();
    let digest = Sha256::digest(backend.as_str().as_bytes());
    format!("{readable}-{:02x}{:02x}", digest[0], digest[1])
}

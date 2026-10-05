//! Fetching a release (`upgrades.md` § Fetch): its server archive and image
//! archive for this server's architecture, each checked against the
//! release's `SHA256SUMS`, unpacked together into a stage under `releases/`
//! and renamed to the version once whole. The asset names and the sums'
//! format are the part of a release every earlier release reads
//! (`builds-and-releases.md` § Release workflow).

use std::{
    io::{self, Read},
    path::{Path, PathBuf},
};

use demi_shared_artifacts::Digest;
use semver::Version;
use tokio_util::sync::CancellationToken;

use crate::layout::{Layout, sync_directory};

/// The repository whose GitHub releases are Demi's.
const REPOSITORY: &str = "wspl/demi";

/// The file of the assets' SHA-256 sums.
const SUMS: &str = "SHA256SUMS";

/// The largest asset a release has, its image archive, is far below this.
const LARGEST: u64 = 4 << 30;

/// Where releases come from.
pub enum Source {
    /// The repository's GitHub releases.
    GitHub,
    /// A directory holding a release's assets, as a developer's build or a
    /// server without internet access provides them.
    Directory(PathBuf),
}

/// This server's architecture, as the assets name it.
pub fn architecture() -> io::Result<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => Ok("amd64"),
        "aarch64" => Ok("arm64"),
        other => Err(io::Error::other(format!("Demi runs on no {other} server"))),
    }
}

fn server_asset(version: &Version, architecture: &str) -> String {
    format!("demi-{version}-server-linux-{architecture}.tar.zst")
}

fn image_asset(version: &Version, architecture: &str) -> String {
    format!("demi-{version}-image-linux-{architecture}.tar")
}

/// The newest release `source` offers.
pub async fn newest(source: &Source) -> io::Result<Version> {
    match source {
        Source::GitHub => {
            #[derive(serde::Deserialize)]
            struct Latest {
                tag_name: String,
            }
            let client = demi_shared_artifacts::client_following_redirects().map_err(io::Error::other)?;
            let response = client
                .get(format!("https://api.github.com/repos/{REPOSITORY}/releases/latest"))
                .header("user-agent", "demi-server")
                .header("accept", "application/vnd.github+json")
                .send()
                .await
                .and_then(|response| response.error_for_status())
                .map_err(|error| io::Error::other(error.without_url().to_string()))?;
            let bytes = response
                .bytes()
                .await
                .map_err(|error| io::Error::other(error.without_url().to_string()))?;
            let latest: Latest = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
            let version = latest.tag_name.strip_prefix('v').ok_or_else(|| {
                io::Error::other(format!("the newest release's tag {} names no version", latest.tag_name))
            })?;
            Version::parse(version).map_err(io::Error::other)
        }
        Source::Directory(directory) => {
            let architecture = architecture()?;
            let suffix = format!("-server-linux-{architecture}.tar.zst");
            let mut versions = Vec::new();
            for entry in std::fs::read_dir(directory)? {
                let name = entry?.file_name().to_string_lossy().into_owned();
                if let Some(version) = name.strip_prefix("demi-").and_then(|name| name.strip_suffix(&suffix)) {
                    versions.push(Version::parse(version).map_err(io::Error::other)?);
                }
            }
            versions.into_iter().max().ok_or_else(|| {
                io::Error::other(format!("{} holds no server release", directory.display()))
            })
        }
    }
}

/// Unpacks `version` into its release directory, unless it is there.
pub async fn fetch(
    layout: &Layout,
    source: &Source,
    version: &Version,
    cancel: &CancellationToken,
) -> io::Result<()> {
    let destination = layout.release(version);
    if destination.exists() {
        return Ok(());
    }
    let releases = layout.releases();
    std::fs::create_dir_all(&releases)?;
    for entry in std::fs::read_dir(&releases)? {
        let entry = entry?;
        // A stage an interrupted fetch left behind.
        if entry.file_name().to_string_lossy().starts_with(".staging-") {
            std::fs::remove_dir_all(entry.path())?;
        }
    }
    let stage = tempfile::Builder::new()
        .prefix(&format!(".staging-{version}-"))
        .tempdir_in(&releases)?;
    let architecture = architecture()?;
    let sums = obtain(source, version, SUMS, stage.path(), cancel).await?;
    let sums = std::fs::read_to_string(sums)?;
    let sums = parse_sums(&sums)?;
    let root = stage.path().join("root");
    std::fs::create_dir(&root)?;
    for asset in [server_asset(version, architecture), image_asset(version, architecture)] {
        let path = obtain(source, version, &asset, stage.path(), cancel).await?;
        let expected = sums
            .iter()
            .find(|(name, _)| *name == asset)
            .map(|(_, sha256)| sha256.as_str())
            .ok_or_else(|| io::Error::other(format!("{SUMS} names no {asset}")))?;
        let found = demi_shared_artifacts::digest(&path, LARGEST, cancel)
            .await
            .map_err(io::Error::other)?;
        if found.sha256 != expected {
            return Err(io::Error::other(format!("{asset} does not match its SHA-256 in {SUMS}")));
        }
        let file = std::fs::File::open(&path)?;
        if asset.ends_with(".zst") {
            unpack(zstd::Decoder::new(file)?, &root)?;
        } else {
            unpack(file, &root)?;
        }
    }
    std::fs::rename(&root, &destination)?;
    sync_directory(&releases)
}

/// The asset `name` of `version` as a file: downloaded into `stage`, or the
/// source directory's own.
async fn obtain(
    source: &Source,
    version: &Version,
    name: &str,
    stage: &Path,
    cancel: &CancellationToken,
) -> io::Result<PathBuf> {
    match source {
        Source::Directory(directory) => Ok(directory.join(name)),
        Source::GitHub => {
            let url = format!("https://github.com/{REPOSITORY}/releases/download/v{version}/{name}");
            let path = stage.join(name);
            let client = demi_shared_artifacts::client_following_redirects().map_err(io::Error::other)?;
            let mut file = tokio::fs::File::create(&path).await?;
            let _: Digest = demi_shared_artifacts::download_measured(&client, &url, LARGEST, &mut file, cancel)
                .await
                .map_err(|error| io::Error::other(format!("{name}: {error}")))?;
            Ok(path)
        }
    }
}

/// The names and SHA-256 sums of `SHA256SUMS`: one `<sum>  <name>` per line.
fn parse_sums(text: &str) -> io::Result<Vec<(&str, String)>> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let (sum, name) = line
                .split_once(char::is_whitespace)
                .ok_or_else(|| io::Error::other(format!("{SUMS} holds a line without a name: {line}")))?;
            let name = name.trim_start().trim_start_matches('*');
            let valid = sum.len() == 64 && sum.bytes().all(|byte| byte.is_ascii_hexdigit());
            if !valid {
                return Err(io::Error::other(format!("{SUMS} holds no SHA-256 for {name}")));
            }
            Ok((name, sum.to_ascii_lowercase()))
        })
        .collect()
}

/// Unpacks a tar stream into `into`, which only regular files, directories
/// and links may enter, none of them leaving it.
fn unpack(reader: impl Read, into: &Path) -> io::Result<()> {
    let mut archive = tar::Archive::new(reader);
    archive.set_preserve_permissions(true);
    for entry in archive.entries()? {
        let mut entry = entry?;
        let kind = entry.header().entry_type();
        let accepted = kind.is_file() || kind.is_dir() || kind.is_symlink() || kind.is_hard_link();
        let path = entry.path()?.into_owned();
        if !accepted {
            return Err(io::Error::other(format!("{} is no file, directory or link", path.display())));
        }
        if !entry.unpack_in(into)? {
            return Err(io::Error::other(format!("{} leaves the release", path.display())));
        }
    }
    Ok(())
}

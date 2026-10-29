//! Fetching a release: its server archive and image archive for this
//! server's architecture, each checked against the release's `SHA256SUMS`,
//! unpacked together into a stage under `releases/` and renamed to the
//! version once whole. The asset names and the sums' format are the part of
//! a release every earlier release reads (`.github/workflows/release.yml`).

use std::{
    io::{self, Read},
    path::{Path, PathBuf},
};

use semver::Version;
use tokio_util::sync::CancellationToken;

use crate::layout::{Layout, sync_directory};

use demi_shared_artifacts::releases::{self, Channel};

/// The file of the assets' SHA-256 sums.
const SUMS: &str = "SHA256SUMS";

/// The largest asset a release has, its image archive, is far below this; a
/// download is written to the disk, which a wrong or hostile source would
/// otherwise fill.
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

/// The newest release of `channel` that `source` offers (`upgrades.md` §
/// The upgrade). A directory's newest is its highest version, whatever the
/// channel.
pub async fn newest(source: &Source, channel: Channel) -> io::Result<Version> {
    match source {
        Source::GitHub => releases::newest(channel).await?.map(|newest| newest.version).ok_or_else(|| {
            io::Error::other(match channel {
                Channel::Beta => "Demi has published no beta release yet: --canary takes the newest build",
                Channel::Canary => "Demi has no release yet",
            })
        }),
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

/// How far a fetch is.
pub enum Fetched {
    /// The release is unpacked already.
    Present,
    /// The bytes downloaded of all the release's, from GitHub.
    Downloading { done: u64, total: u64 },
    /// Every asset is here: `bytes` downloaded, none from a directory.
    Downloaded { bytes: u64 },
    /// The archives' bytes checked and unpacked, of all.
    Unpacking { done: u64, total: u64 },
    /// The release is unpacked, from archives of `bytes`.
    Unpacked { bytes: u64 },
}

/// Downloads `version`'s assets, then checks and unpacks them into its
/// release directory, unless it is there; `progress` hears how far it is.
pub async fn fetch(
    layout: &Layout,
    source: &Source,
    version: &Version,
    cancel: &CancellationToken,
    progress: &mut (dyn FnMut(Fetched) + Send),
) -> io::Result<()> {
    let destination = layout.release(version);
    if destination.exists() {
        progress(Fetched::Present);
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
    let archives = [server_asset(version, architecture), image_asset(version, architecture)];
    let names = [SUMS.to_owned(), archives[0].clone(), archives[1].clone()];
    let mut downloads = Downloads::new(source, version, &names).await?;
    crate::data::ensure_room(&releases, downloads.bytes, &format!("the download of {version}"))?;
    let mut downloaded = |done, total| progress(Fetched::Downloading { done, total });
    let sums = obtain(source, version, SUMS, stage.path(), cancel, &mut downloads, &mut downloaded).await?;
    let mut paths = Vec::new();
    for asset in &archives {
        paths.push(obtain(source, version, asset, stage.path(), cancel, &mut downloads, &mut downloaded).await?);
    }
    progress(Fetched::Downloaded { bytes: downloads.done });
    let sums = std::fs::read_to_string(sums)?;
    let sums = parse_sums(&sums)?;
    let mut unpacked = 0;
    for (asset, path) in archives.iter().zip(&paths) {
        let file = std::fs::File::open(path)?;
        unpacked += if asset.ends_with(".zst") { unpacked_bytes(zstd::Decoder::new(file)?)? } else { unpacked_bytes(file)? };
    }
    crate::data::ensure_room(&releases, unpacked, &format!("unpacking {version}"))?;
    let root = stage.path().join("root");
    std::fs::create_dir(&root)?;
    let total = paths.iter().map(|path| std::fs::metadata(path).map(|metadata| metadata.len())).sum::<io::Result<u64>>()?;
    let mut done = 0;
    for (asset, path) in archives.iter().zip(&paths) {
        let expected = sums
            .iter()
            .find(|(name, _)| name == asset)
            .map(|(_, sha256)| sha256.as_str())
            .ok_or_else(|| io::Error::other(format!("{SUMS} names no {asset}")))?;
        let found = demi_shared_artifacts::digest(path, LARGEST, cancel)
            .await
            .map_err(io::Error::other)?;
        if found.sha256 != expected {
            return Err(io::Error::other(format!("{asset} does not match its SHA-256 in {SUMS}")));
        }
        let file = Counted {
            inner: std::fs::File::open(path)?,
            done: &mut done,
            total,
            progress: &mut *progress,
        };
        if asset.ends_with(".zst") {
            unpack(zstd::Decoder::new(file)?, &root)?;
        } else {
            unpack(file, &root)?;
        }
    }
    std::fs::rename(&root, &destination)?;
    sync_directory(&releases)?;
    progress(Fetched::Unpacked { bytes: total });
    Ok(())
}

/// An archive that tells how many of all the archives' bytes are read, at
/// most once a megabyte.
struct Counted<'a, R> {
    inner: R,
    done: &'a mut u64,
    total: u64,
    progress: &'a mut (dyn FnMut(Fetched) + Send),
}

impl<R: Read> Read for Counted<'_, R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let read = self.inner.read(buffer)?;
        let before = *self.done;
        *self.done += read as u64;
        if before / (1 << 20) != *self.done / (1 << 20) {
            (self.progress)(Fetched::Unpacking {
                done: *self.done,
                total: self.total,
            });
        }
        Ok(read)
    }
}

/// The asset `name` of `version` as a file: downloaded into `stage`, or the
/// source directory's own.
async fn obtain(
    source: &Source,
    version: &Version,
    name: &str,
    stage: &Path,
    cancel: &CancellationToken,
    downloads: &mut Downloads,
    progress: &mut (dyn FnMut(u64, u64) + Send),
) -> io::Result<PathBuf> {
    match source {
        Source::Directory(directory) => Ok(directory.join(name)),
        Source::GitHub => {
            let url = releases::asset_url(version, name);
            let path = stage.join(name);
            let client = demi_shared_artifacts::download_client().map_err(io::Error::other)?;
            // GitHub serves a release's assets through a redirect to
            // short-lived storage; the sums check what arrives.
            let request = demi_shared_artifacts::Download {
                limit: LARGEST,
                redirects: releases::REDIRECTS,
                ..demi_shared_artifacts::Download::new(&url)
            };
            let (before, bytes) = (downloads.done, downloads.bytes);
            demi_shared_artifacts::download(&client, request, &path, cancel, |received, _| {
                progress(before + received, bytes);
            })
            .await
            .and_then(|received| received.plain())
            .map_err(|error| io::Error::other(format!("{name}: {error}")))?;
            downloads.done += std::fs::metadata(&path)?.len();
            Ok(path)
        }
    }
}

/// The bytes a fetch downloads, and how many of them it has.
struct Downloads {
    done: u64,
    bytes: u64,
}

impl Downloads {
    /// The sizes of the assets `names` of `version`, asked of the addresses
    /// they download from, which answer as the downloads would; nothing to
    /// download from a directory.
    async fn new(source: &Source, version: &Version, names: &[String]) -> io::Result<Self> {
        let mut bytes = 0;
        if let Source::GitHub = source {
            for name in names {
                bytes += releases::asset_size(version, name).await?;
            }
        }
        Ok(Self { done: 0, bytes })
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

/// The bytes a tar stream's members take once unpacked, each in whole
/// blocks of 4 KiB with a block for its own record.
fn unpacked_bytes(reader: impl Read) -> io::Result<u64> {
    const BLOCK: u64 = 4096;
    let mut archive = tar::Archive::new(reader);
    let mut bytes = 0u64;
    for entry in archive.entries()? {
        bytes = bytes.saturating_add(entry?.size().div_ceil(BLOCK).saturating_add(1).saturating_mul(BLOCK));
    }
    Ok(bytes)
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

//! `cargo xtask cloud-image package` (`images.md` § Build pipeline): the
//! second stage of a Cloud image build, which
//! `packages/guest-image/rootfs/build.sh` runs as root on a Linux builder of
//! the image's architecture, with an `xtask` built for that architecture.
//! Into the Ubuntu tree the script made, it installs the runner and its
//! `demi` alias from the verified runner release, each command package from
//! its verified release under its content-addressed path, the pinned Chrome
//! for Testing through `artifact`'s archive installation and the pinned uv.
//! It reads the package inventory from the tree's dpkg database without
//! running a program of the image, writes the root archive with GNU tar,
//! checks the manifest the way the machine manager decodes it, publishes the
//! release directory through `artifact`'s release publication and prints the
//! base version.

use std::collections::BTreeMap;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::Stdio;

use demi_artifact::{Archive, Digest, Mode, Permissions, Publication, ReleaseFile, ReleaseRecord, Staged};
use demi_builtin_protocol::release::{BrowserRelease, IMAGE_BROWSERS};
use demi_command_service::protocol::{PackageArtifact, PackageDescriptor};
use demi_machines_protocol::image::{
    ARTIFACTS_PATH, Architecture, CloudImageManifest, FormatVersion, INIT_PATH, InstalledPackage, ManifestError,
    Os, RUNNER_PATH, RootfsArchive, RootfsFile, StandaloneTool,
};
use demi_runner_protocol::release::RunnerRelease;
use serde::Deserialize;
use tokio_util::sync::CancellationToken;

use crate::native::{DESCRIPTOR, MANIFEST};

/// The uv release every image installs, pinned in the repository. `xtask`
/// carries the pin it was built with.
const UV: &str = include_str!("../../../packages/guest-image/rootfs/uv.json");
/// The image release's record.
const IMAGE_MANIFEST: &str = "manifest.json";
/// The name the runner also answers to, beside it in `/usr/bin`.
const RUNNER_ALIAS: &str = "demi";
/// Where the image's standalone tools are installed.
const TOOLS_PATH: &str = "/usr/local/bin";
/// What the build reads of the tree: its dpkg database and its os-release
/// file.
const DPKG_STATUS: &str = "/var/lib/dpkg/status";
const OS_RELEASE: &str = "/usr/lib/os-release";

#[derive(clap::Subcommand)]
pub enum Command {
    /// Completes the Cloud image release of the tree rootfs/build.sh made,
    /// and publishes it.
    Package(Options),
}

#[derive(clap::Args)]
pub struct Options {
    /// The Ubuntu tree rootfs/build.sh made.
    #[arg(long, value_name = "DIRECTORY")]
    root: PathBuf,
    /// The runner releases `cargo xtask native package --package
    /// demi-runner` published; the image embeds the one their manifest
    /// names.
    #[arg(long, value_name = "DIRECTORY")]
    runners: PathBuf,
    /// A command package release to embed; repeat for each package.
    #[arg(long = "package", value_name = "DIRECTORY", required = true)]
    packages: Vec<PathBuf>,
    /// The release directory to publish, a new one for every build.
    #[arg(long, value_name = "DIRECTORY")]
    output: PathBuf,
}

/// Why an image could not be packaged.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("a Cloud image is packaged on a Linux builder of its architecture")]
    NotLinux,
    #[error("the pinned {name} release is invalid: {reason}")]
    Pin { name: &'static str, reason: String },
    #[error("{}: {source}", path.display())]
    File { path: PathBuf, source: demi_artifact::Error },
    #[error("{} {reason}", path.display())]
    Invalid { path: PathBuf, reason: String },
    #[error("{release} carries nothing for {target}")]
    NotCarried { release: String, target: &'static str },
    #[error("the command package {0} is named twice")]
    Twice(String),
    #[error("the uv archive {url} {reason}")]
    Uv { url: String, reason: String },
    #[error("dpkg lists {package} as \"{status}\": its installation did not finish")]
    Unfinished { package: String, status: String },
    #[error("tar failed: {0}")]
    Tar(std::process::ExitStatus),
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    #[error(transparent)]
    Artifact(#[from] demi_artifact::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub fn run(command: Command) -> Result<(), Error> {
    let Command::Package(options) = command;
    // The builder's architecture is the image's, and xtask runs on it.
    let architecture = Architecture::host()
        .filter(|_| cfg!(target_os = "linux"))
        .ok_or(Error::NotLinux)?;
    let pins = Pins::pinned()?;
    let base = crate::interruptible(|cancel| async move {
        let client = demi_artifact::client()?;
        package(&options, architecture, &pins, &client, &cancel).await
    })??;
    println!("{base}");
    Ok(())
}

/// What an image downloads, as the repository pins it.
struct Pins {
    chrome: BrowserRelease,
    uv: UvRelease,
}

impl Pins {
    fn pinned() -> Result<Self, Error> {
        let chrome = BrowserRelease::pinned().map_err(|error| Error::Pin {
            name: "Chrome for Testing",
            reason: error.to_string(),
        })?;
        let uv = UvRelease::parse(UV).map_err(|reason| Error::Pin { name: "uv", reason })?;
        Ok(Self { chrome, uv })
    }
}

/// The pinned uv release (`packages/guest-image/rootfs/uv.json`): its
/// version and the archive for each image architecture.
#[derive(Debug, Deserialize, garde::Validate)]
#[serde(deny_unknown_fields)]
struct UvRelease {
    #[garde(length(min = 1))]
    version: String,
    #[garde(dive)]
    amd64: UvArchive,
    #[garde(dive)]
    arm64: UvArchive,
}

/// One architecture's uv archive, a gzip-compressed tar file: where it is
/// downloaded from, its size and SHA-256, and the paths of its executables,
/// each installed in `/usr/local/bin` under its file name.
#[derive(Debug, Deserialize, garde::Validate)]
#[serde(deny_unknown_fields)]
struct UvArchive {
    #[garde(url)]
    url: String,
    #[garde(range(min = 1))]
    size: u64,
    #[garde(custom(demi_command_service::protocol::digest))]
    sha256: String,
    #[garde(length(min = 1), inner(length(min = 1)))]
    executables: Vec<String>,
}

impl UvRelease {
    fn parse(json: &str) -> Result<Self, String> {
        let release: Self = serde_json::from_str(json).map_err(|error| error.to_string())?;
        garde::Validate::validate(&release).map_err(|report| report.to_string().trim_end().to_owned())?;
        Ok(release)
    }

    fn archive(&self, architecture: Architecture) -> &UvArchive {
        match architecture {
            Architecture::Amd64 => &self.amd64,
            Architecture::Arm64 => &self.arm64,
        }
    }
}

/// Completes the image of the tree at `options.root` for `architecture`,
/// publishes it at `options.output` and returns its base version. The tree
/// and the release records are read before anything is installed, so a bad
/// input fails before the downloads.
async fn package(
    options: &Options,
    architecture: Architecture,
    pins: &Pins,
    client: &demi_artifact::Client,
    cancel: &CancellationToken,
) -> Result<String, Error> {
    let root = std::path::absolute(&options.root)?;
    let output = std::path::absolute(&options.output)?;
    let target = architecture.target();
    let packages = installed_packages(&root).await?;
    let ubuntu = ubuntu_release(&root).await?;
    let (runner, runner_artifact) = runner_release(&options.runners, target).await?;
    let mut releases: Vec<(PackageDescriptor, PackageArtifact)> = Vec::new();
    for directory in &options.packages {
        let (descriptor, artifact) = package_release(directory, target).await?;
        if releases.iter().any(|(release, _)| release.id == descriptor.id) {
            return Err(Error::Twice(descriptor.id));
        }
        releases.push((descriptor, artifact));
    }

    let mut executables = BTreeMap::new();
    install_runner(&root, &options.runners, &runner, &runner_artifact, target, cancel).await?;
    executables.insert(RUNNER_PATH.to_owned(), runner_artifact);
    for (directory, (descriptor, artifact)) in options.packages.iter().zip(&releases) {
        let path = install_package(&root, directory, descriptor, artifact, target, cancel).await?;
        executables.insert(path, artifact.clone());
    }
    let (chrome_path, chrome, chrome_tool) = install_chrome(&root, &pins.chrome, target, client, cancel).await?;
    executables.insert(chrome_path, chrome);
    let (uv, uv_tool) = install_uv(&root, &pins.uv, architecture, client, cancel).await?;
    executables.extend(uv);
    executables.insert(INIT_PATH.to_owned(), measure(&in_tree(&root, INIT_PATH), cancel).await?);

    let parent = output.parent().expect("an absolute release directory has a parent");
    tokio::fs::create_dir_all(parent).await.map_err(at(parent))?;
    // The archive is written beside the release, then published into it.
    let written = tempfile::Builder::new().prefix(".cloud-image-").tempdir_in(parent)?;
    let archive = written.path().join(RootfsFile::TarZst.name());
    eprintln!("Cloud image: writing {}", RootfsFile::TarZst.name());
    let writing = {
        let root = root.clone();
        let archive = archive.clone();
        let cancel = cancel.clone();
        tokio::task::spawn_blocking(move || write_archive(&root, &archive, &cancel))
    };
    writing.await.map_err(std::io::Error::other)??;
    let digest = demi_artifact::digest(&archive, u64::MAX, cancel).await?;
    let manifest = CloudImageManifest {
        format_version: FormatVersion,
        os: Os::Linux,
        architecture,
        rootfs: RootfsArchive {
            sha256: digest.sha256.clone(),
            size: digest.size,
            file: RootfsFile::TarZst,
        },
        ubuntu,
        packages,
        executables,
        releases: releases.into_iter().map(|(descriptor, _)| descriptor).collect(),
        runner,
        tools: vec![uv_tool, chrome_tool],
    };
    let bytes = crate::record(&manifest).map_err(ManifestError::from)?;
    // The manifest is checked the way the manager decodes it.
    CloudImageManifest::decode(&bytes)?;
    let record = ReleaseRecord {
        name: IMAGE_MANIFEST,
        bytes: &bytes,
    };
    let files = [ReleaseFile {
        source: archive,
        path: PathBuf::from(RootfsFile::TarZst.name()),
        digest,
        executable: false,
    }];
    demi_artifact::publish_release(&output, record, &files, cancel).await?;
    eprintln!("Cloud image: {}", output.display());
    // The base version names the manifest's bytes as published.
    let published = demi_artifact::digest(&output.join(IMAGE_MANIFEST), u64::MAX, cancel).await?;
    Ok(published.sha256)
}

/// The runner release the manifest of `runners` names, and its executable
/// for `target`.
async fn runner_release(runners: &Path, target: &'static str) -> Result<(RunnerRelease, PackageArtifact), Error> {
    let pointer = runners.join(MANIFEST);
    let bytes = tokio::fs::read(&pointer).await.map_err(at(&pointer))?;
    let release = RunnerRelease::decode(&bytes).map_err(|error| invalid(&pointer, format!("is invalid: {error}")))?;
    let artifact = release
        .targets
        .get(target)
        .cloned()
        .ok_or_else(|| Error::NotCarried {
            release: format!("the runner release {}", release.release),
            target,
        })?;
    Ok((release, artifact))
}

/// The command package release at `directory`, and its executable for
/// `target`.
async fn package_release(directory: &Path, target: &'static str) -> Result<(PackageDescriptor, PackageArtifact), Error> {
    let path = directory.join(DESCRIPTOR);
    let bytes = tokio::fs::read(&path).await.map_err(at(&path))?;
    let value = serde_json::from_slice(&bytes).map_err(|error| invalid(&path, format!("is invalid: {error}")))?;
    let descriptor = PackageDescriptor::parse(value).map_err(|error| invalid(&path, format!("is invalid: {error}")))?;
    let artifact = descriptor
        .targets
        .get(target)
        .cloned()
        .ok_or_else(|| Error::NotCarried {
            release: format!("the command package {}@{}", descriptor.id, descriptor.version),
            target,
        })?;
    Ok((descriptor, artifact))
}

/// The executable a release carries for `target`: the one file in the
/// release's directory of that target (`builds-and-releases.md`
/// § Packaging).
async fn release_executable(release: &Path, target: &str) -> Result<PathBuf, Error> {
    let directory = release.join(target);
    let mut entries = tokio::fs::read_dir(&directory).await.map_err(at(&directory))?;
    let mut files = Vec::new();
    while let Some(entry) = entries.next_entry().await.map_err(at(&directory))? {
        files.push(entry.path());
    }
    match <[PathBuf; 1]>::try_from(files) {
        Ok([file]) => Ok(file),
        Err(_) => Err(invalid(&directory, "does not hold exactly one executable")),
    }
}

/// Installs the runner of `release`, one of the runner releases in
/// `runners`, as `/usr/bin/demi-runner`, and `demi` as its alias.
async fn install_runner(
    root: &Path,
    runners: &Path,
    release: &RunnerRelease,
    artifact: &PackageArtifact,
    target: &str,
    cancel: &CancellationToken,
) -> Result<(), Error> {
    let source = release_executable(&runners.join(&release.release), target).await?;
    let installed = in_tree(root, RUNNER_PATH);
    install_executable(&source, artifact, &installed, cancel).await?;
    // `demi` is the runner by another name, beside it.
    let name = installed.file_name().expect("the runner's path names a file");
    symlink(name, &installed.with_file_name(RUNNER_ALIAS)).await?;
    eprintln!("Cloud image: runner release {}", release.release);
    Ok(())
}

/// Installs the executable of `descriptor`'s release at `directory` under
/// its content-addressed path, and returns that path in the image.
async fn install_package(
    root: &Path,
    directory: &Path,
    descriptor: &PackageDescriptor,
    artifact: &PackageArtifact,
    target: &str,
    cancel: &CancellationToken,
) -> Result<String, Error> {
    let source = release_executable(directory, target).await?;
    let name = source
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| invalid(&source, "is not named in UTF-8"))?;
    let path = format!("{ARTIFACTS_PATH}/{}/{name}", artifact.sha256);
    let installed = in_tree(root, &path);
    let parent = installed.parent().expect("an artifact's path has a directory");
    tokio::fs::create_dir_all(parent).await.map_err(at(parent))?;
    install_executable(&source, artifact, &installed, cancel).await?;
    eprintln!("Cloud image: command package {}@{}", descriptor.id, descriptor.version);
    Ok(path)
}

/// Copies the executable at `source` to `destination`, checking its bytes
/// against `artifact` as they are copied: nothing is at `destination` unless
/// they match.
async fn install_executable(
    source: &Path,
    artifact: &PackageArtifact,
    destination: &Path,
    cancel: &CancellationToken,
) -> Result<(), Error> {
    let expected = Digest {
        size: artifact.size,
        sha256: artifact.sha256.clone(),
    };
    let mut input = tokio::fs::File::open(source).await.map_err(at(source))?;
    let publication = Publication {
        mode: Mode::CreateNew,
        permissions: Permissions::Executable,
        durable: false,
    };
    let mut staged = Staged::new(destination, publication).await.map_err(at(destination))?;
    demi_artifact::copy(&mut input, &expected, staged.file(), cancel)
        .await
        .map_err(at(source))?;
    staged.publish().await.map_err(at(destination))
}

/// Makes `alias` a symbolic link to `target`, where images are built.
#[cfg(unix)]
async fn symlink(target: &std::ffi::OsStr, alias: &Path) -> Result<(), Error> {
    tokio::fs::symlink(target, alias).await.map_err(at(alias))
}

/// Windows has no image build: `run` refuses it before this is reached.
#[cfg(not(unix))]
async fn symlink(_: &std::ffi::OsStr, _: &Path) -> Result<(), Error> {
    Err(Error::NotLinux)
}

/// Installs the pinned Chrome for Testing archive of `target` where
/// `demi-commands` looks for it first; returns its executable's path in the
/// image with its size and SHA-256, and the tool's entry.
async fn install_chrome(
    root: &Path,
    release: &BrowserRelease,
    target: &'static str,
    client: &demi_artifact::Client,
    cancel: &CancellationToken,
) -> Result<(String, PackageArtifact, StandaloneTool), Error> {
    let platform = release
        .platforms
        .iter()
        .find(|platform| platform.target == target)
        .ok_or_else(|| Error::NotCarried {
            release: format!("Chrome for Testing {}", release.version),
            target,
        })?;
    let archive = Archive {
        url: platform.url.clone(),
        digest: Digest {
            size: platform.size,
            sha256: platform.sha256.clone(),
        },
        executable: platform.executable.clone(),
    };
    let browsers = in_tree(root, IMAGE_BROWSERS);
    let executable = demi_artifact::install_archive(client, &browsers, &archive, cancel).await?;
    // The installation's lock serves installers running at once on one
    // machine; an image starts with none running.
    let lock = browsers.join(format!("{}.lock", platform.sha256));
    tokio::fs::remove_file(&lock).await.map_err(at(&lock))?;
    let path = format!("{IMAGE_BROWSERS}/{}/{}", platform.sha256, platform.executable);
    let measured = measure(&executable, cancel).await?;
    let tool = StandaloneTool {
        name: "chrome".to_owned(),
        version: release.version.clone(),
        sha256: platform.sha256.clone(),
    };
    eprintln!("Cloud image: Chrome for Testing {}", release.version);
    Ok((path, measured, tool))
}

/// Downloads the pinned uv archive of `architecture` and installs its
/// executables in `/usr/local/bin`; returns their paths in the image with
/// their sizes and SHA-256s, and the tool's entry.
async fn install_uv(
    root: &Path,
    release: &UvRelease,
    architecture: Architecture,
    client: &demi_artifact::Client,
    cancel: &CancellationToken,
) -> Result<(Vec<(String, PackageArtifact)>, StandaloneTool), Error> {
    let archive = release.archive(architecture);
    let expected = Digest {
        size: archive.size,
        sha256: archive.sha256.clone(),
    };
    let downloads = tempfile::tempdir()?;
    let downloaded = downloads.path().join("uv.tar.gz");
    let mut file = tokio::fs::File::create(&downloaded).await?;
    demi_artifact::download(client, &archive.url, &expected, &mut file, cancel).await?;
    drop(file);
    let tools = in_tree(root, TOOLS_PATH);
    let unpacking = {
        let url = archive.url.clone();
        let executables = archive.executables.clone();
        let tools = tools.clone();
        tokio::task::spawn_blocking(move || unpack(&url, &downloaded, &executables, &tools))
    };
    let names = unpacking.await.map_err(std::io::Error::other)??;
    let mut installed = Vec::new();
    for name in names {
        installed.push((format!("{TOOLS_PATH}/{name}"), measure(&tools.join(&name), cancel).await?));
    }
    let tool = StandaloneTool {
        name: "uv".to_owned(),
        version: release.version.clone(),
        sha256: archive.sha256.clone(),
    };
    eprintln!("Cloud image: uv {}", release.version);
    Ok((installed, tool))
}

/// Installs the regular files at `paths` in the gzip-compressed tar archive
/// at `archive`, downloaded from `url`, as executables in `directory`, each
/// under its file name, and returns those names.
fn unpack(url: &str, archive: &Path, paths: &[String], directory: &Path) -> Result<Vec<String>, Error> {
    let fails = |reason: String| Error::Uv {
        url: url.to_owned(),
        reason,
    };
    let file = std::fs::File::open(archive)?;
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(file));
    let mut found: Vec<&String> = Vec::new();
    let mut names = Vec::new();
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        let Some(named) = paths.iter().find(|named| Path::new(named) == path) else {
            continue;
        };
        if !entry.header().entry_type().is_file() {
            return Err(fails(format!("holds {named} as something other than a file")));
        }
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| fails(format!("names {named} without a file name")))?
            .to_owned();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes)?;
        let publication = Publication {
            mode: Mode::CreateNew,
            permissions: Permissions::Executable,
            durable: false,
        };
        demi_artifact::publish_bytes_blocking(&directory.join(&name), &bytes, publication)
            .map_err(at(&directory.join(&name)))?;
        found.push(named);
        names.push(name);
    }
    if let Some(missing) = paths.iter().find(|named| !found.contains(named)) {
        return Err(fails(format!("holds no {missing}")));
    }
    Ok(names)
}

/// The size and SHA-256 of the file at `path` in the tree, which must be a
/// regular file rather than a link, which could lead out of the tree.
async fn measure(path: &Path, cancel: &CancellationToken) -> Result<PackageArtifact, Error> {
    let metadata = tokio::fs::symlink_metadata(path).await.map_err(at(path))?;
    if !metadata.is_file() {
        return Err(invalid(path, "is not a regular file"));
    }
    let digest = demi_artifact::digest(path, u64::MAX, cancel).await.map_err(at(path))?;
    Ok(PackageArtifact {
        sha256: digest.sha256,
        size: digest.size,
    })
}

/// The fields of one package in the dpkg database that the inventory reads.
#[derive(Default)]
struct Stanza<'a> {
    package: Option<&'a str>,
    status: Option<&'a str>,
    version: Option<&'a str>,
}

/// The packages the tree's dpkg database lists as installed, with their
/// versions, read from the database's file. A package removed with its
/// configuration kept is not installed; a package whose installation did
/// not finish fails the image.
async fn installed_packages(root: &Path) -> Result<Vec<InstalledPackage>, Error> {
    let path = in_tree(root, DPKG_STATUS);
    let database = tokio::fs::read_to_string(&path).await.map_err(at(&path))?;
    let mut packages = Vec::new();
    let mut stanza = Stanza::default();
    // A blank line ends a package's stanza, and so does the file's end.
    for line in database.lines().chain([""]) {
        if line.trim().is_empty() {
            let ended = std::mem::take(&mut stanza);
            if let Some(package) = installed(ended, &path)? {
                packages.push(package);
            }
            continue;
        }
        // A line that starts with white space continues the field before it.
        if line.starts_with([' ', '\t']) {
            continue;
        }
        let (field, value) = line
            .split_once(':')
            .ok_or_else(|| invalid(&path, format!("has a line without a field: {line}")))?;
        let value = Some(value.trim());
        match field {
            "Package" => stanza.package = value,
            "Status" => stanza.status = value,
            "Version" => stanza.version = value,
            _ => {}
        }
    }
    Ok(packages)
}

/// The package of `stanza` when dpkg lists it as installed; none for an
/// empty stanza or a package that is not installed.
fn installed(stanza: Stanza<'_>, database: &Path) -> Result<Option<InstalledPackage>, Error> {
    let (package, status) = match stanza {
        Stanza {
            package: None,
            status: None,
            version: None,
        } => return Ok(None),
        Stanza {
            package: Some(package),
            status: Some(status),
            ..
        } => (package, status),
        _ => return Err(invalid(database, "lists a package without its name or status")),
    };
    // The status is the wanted action, a flag and the package's state.
    let words: Vec<&str> = status.split_whitespace().collect();
    match words.as_slice() {
        [_, "ok", "installed"] => {}
        [_, "ok", "not-installed" | "config-files"] => return Ok(None),
        _ => {
            return Err(Error::Unfinished {
                package: package.to_owned(),
                status: status.to_owned(),
            });
        }
    }
    let version = stanza
        .version
        .ok_or_else(|| invalid(database, format!("lists {package} without its version")))?;
    Ok(Some(InstalledPackage {
        name: package.to_owned(),
        version: version.to_owned(),
    }))
}

/// The tree's Ubuntu release, such as `26.04`: `VERSION_ID` in its
/// os-release file.
async fn ubuntu_release(root: &Path) -> Result<String, Error> {
    let path = in_tree(root, OS_RELEASE);
    let text = tokio::fs::read_to_string(&path).await.map_err(at(&path))?;
    text.lines()
        .find_map(|line| line.strip_prefix("VERSION_ID="))
        .map(|value| value.trim_matches(['"', '\'']).to_owned())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| invalid(&path, "names no VERSION_ID"))
}

/// Writes the archive of the tree at `root` to a new file at `archive`: GNU
/// tar makes it, keeping numeric owners, ACLs and extended attributes, and
/// zstd compresses tar's output on its way to the file.
fn write_archive(root: &Path, archive: &Path, cancel: &CancellationToken) -> Result<(), Error> {
    let mut tar = std::process::Command::new("tar")
        .args(["--numeric-owner", "--xattrs", "--acls", "-cf", "-", "-C"])
        .arg(root)
        .arg(".")
        .stdout(Stdio::piped())
        .spawn()?;
    let output = tar.stdout.take().expect("tar's output is piped");
    let compressed = compress(output, archive, cancel);
    if compressed.is_err() {
        // tar may still be writing. The kill fails only once tar has
        // exited, and the wait below reaps it either way.
        let _stopped = tar.kill();
    }
    let status = tar.wait()?;
    compressed?;
    if !status.success() {
        return Err(Error::Tar(status));
    }
    Ok(())
}

/// Compresses what `input` yields into a new file at `archive` with zstd,
/// until `input` ends or `cancel` fires.
fn compress(mut input: impl std::io::Read, archive: &Path, cancel: &CancellationToken) -> Result<(), Error> {
    let file = std::fs::File::create_new(archive).map_err(at(archive))?;
    // Level 0 is zstd's default level.
    let mut encoder = zstd::stream::write::Encoder::new(file, 0)?;
    let mut buffer = vec![0; 1024 * 1024];
    loop {
        if cancel.is_cancelled() {
            return Err(demi_artifact::Error::Cancelled.into());
        }
        let count = match input.read(&mut buffer) {
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.into()),
        };
        if count == 0 {
            break;
        }
        encoder.write_all(&buffer[..count])?;
    }
    encoder.finish()?;
    Ok(())
}

/// Where the image's absolute `path` is in the tree at `root`.
fn in_tree(root: &Path, path: &str) -> PathBuf {
    root.join(path.trim_start_matches('/'))
}

/// An error about the file at `path`.
fn at<E: Into<demi_artifact::Error>>(path: &Path) -> impl FnOnce(E) -> Error {
    let path = path.to_owned();
    move |source| Error::File {
        path,
        source: source.into(),
    }
}

/// The file at `path` is not what the build needs, for `reason`.
fn invalid(path: &Path, reason: impl ToString) -> Error {
    Error::Invalid {
        path: path.to_owned(),
        reason: reason.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use demi_artifact::testing::{Answer, Server, zip};
    use demi_builtin_protocol::release::ReleasePlatform;

    use super::*;

    /// The tests build an arm64 image, as for the Lima VM of an Apple
    /// silicon Mac.
    const ARCHITECTURE: Architecture = Architecture::Arm64;
    const TARGET: &str = "aarch64-unknown-linux-musl";
    /// Chrome's executable in its archive.
    const CHROME: &str = "chrome-linux-arm64/chrome";
    /// uv's executables in its archive.
    const UV_EXECUTABLES: [&str; 2] = ["uv-aarch64-unknown-linux-gnu/uv", "uv-aarch64-unknown-linux-gnu/uvx"];

    /// A dpkg database as a build leaves it: two installed packages, and one
    /// removed with its configuration kept, which is not installed.
    const DATABASE: &str = "\
Package: base-files
Status: install ok installed
Priority: required
Version: 14ubuntu1
Description: Debian base system miscellaneous files
 This package contains the basic filesystem hierarchy of a Debian system.

Package: nano
Status: deinstall ok config-files
Version: 8.4-1
Conffiles:
 /etc/nanorc 1f2c1b6a8d2c3c0a7e4d6f5b9a8c7d6e

Package: tini
Status: install ok installed
Architecture: arm64
Version: 0.19.0-3
";

    fn write(path: &Path, bytes: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    /// The size and SHA-256 of `bytes`, as a release records them.
    async fn measured(bytes: &[u8]) -> PackageArtifact {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("measured");
        std::fs::write(&path, bytes).unwrap();
        let digest = demi_artifact::digest(&path, u64::MAX, &CancellationToken::new()).await.unwrap();
        PackageArtifact {
            sha256: digest.sha256,
            size: digest.size,
        }
    }

    /// A gzip-compressed tar archive of `entries`, each a path and its bytes.
    fn gzip_tar(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        let mut builder = tar::Builder::new(encoder);
        for (path, bytes) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder.append_data(&mut header, path, *bytes).unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap()
    }

    /// A command package release at `directory` whose descriptor records
    /// `recorded` as its executable `name`, and whose file holds `bytes`.
    async fn command_package(directory: &Path, id: &str, name: &str, recorded: &[u8], bytes: &[u8]) -> PackageDescriptor {
        let descriptor = PackageDescriptor {
            id: id.to_owned(),
            version: "0.1.3".to_owned(),
            protocol_version: demi_command_service::protocol::VERSION,
            operations: vec!["file.read".to_owned()],
            targets: BTreeMap::from([(TARGET.to_owned(), measured(recorded).await)]),
        };
        write(&directory.join(TARGET).join(name), bytes);
        write(&directory.join(DESCRIPTOR), &crate::record(&descriptor).unwrap());
        descriptor
    }

    /// A build's inputs: a tree as rootfs/build.sh leaves it with the dpkg
    /// database `database`, a runner release, the two command packages, and
    /// the pinned archives on a fixture server. The demi-commands file in its
    /// release holds `commands`, whatever its descriptor records.
    struct Fixture {
        _directory: tempfile::TempDir,
        _server: Server,
        options: Options,
        pins: Pins,
        runner: RunnerRelease,
        releases: Vec<PackageDescriptor>,
    }

    impl Fixture {
        async fn new(database: &str, commands: &[u8]) -> Self {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path();
            let root = path.join("root");
            write(&root.join("usr/bin/tini"), b"tini");
            write(&root.join("usr/lib/os-release"), b"NAME=\"Ubuntu\"\nVERSION_ID=\"26.04\"\nID=ubuntu\n");
            write(&root.join("var/lib/dpkg/status"), database.as_bytes());
            std::fs::create_dir_all(root.join("usr/local/bin")).unwrap();
            let runners = path.join("runners");
            let runner = RunnerRelease {
                release: "1".repeat(64),
                wire: demi_runner_protocol::wire::VERSION,
                command_protocol: demi_command_service::protocol::VERSION,
                targets: BTreeMap::from([(TARGET.to_owned(), measured(b"runner").await)]),
            };
            let record = crate::record(&runner).unwrap();
            write(&runners.join(&runner.release).join(TARGET).join("demi-runner"), b"runner");
            write(&runners.join(&runner.release).join(MANIFEST), &record);
            write(&runners.join(MANIFEST), &record);
            let builtin = path.join("demi-builtin");
            let claude = path.join("demi-claude");
            let releases = vec![
                command_package(&builtin, demi_builtin_protocol::PACKAGE, "demi-commands", b"commands", commands).await,
                command_package(&claude, demi_claude_protocol::PACKAGE, "demi-claude", b"claude", b"claude").await,
            ];
            let chrome = zip(&[(CHROME, b"chrome"), ("chrome-linux-arm64/LICENSE", b"license")]);
            let uv = gzip_tar(&[(UV_EXECUTABLES[0], b"uv"), (UV_EXECUTABLES[1], b"uvx")]);
            let chrome_artifact = measured(&chrome).await;
            let uv_artifact = measured(&uv).await;
            let server = Server::start([
                ("/chrome.zip".to_owned(), Answer::ok(chrome)),
                ("/uv.tar.gz".to_owned(), Answer::ok(uv)),
            ])
            .await;
            let uv_archive = || UvArchive {
                url: server.url("/uv.tar.gz"),
                size: uv_artifact.size,
                sha256: uv_artifact.sha256.clone(),
                executables: UV_EXECUTABLES.map(String::from).to_vec(),
            };
            let pins = Pins {
                chrome: BrowserRelease {
                    version: "153.0.8010.36".to_owned(),
                    platforms: vec![ReleasePlatform {
                        target: TARGET.to_owned(),
                        url: server.url("/chrome.zip"),
                        size: chrome_artifact.size,
                        sha256: chrome_artifact.sha256,
                        executable: CHROME.to_owned(),
                    }],
                },
                uv: UvRelease {
                    version: "0.12.13".to_owned(),
                    amd64: uv_archive(),
                    arm64: uv_archive(),
                },
            };
            let options = Options {
                root,
                runners,
                packages: vec![builtin, claude],
                output: path.join("releases/build"),
            };
            Self {
                _directory: directory,
                _server: server,
                options,
                pins,
                runner,
                releases,
            }
        }
    }

    /// Each entry of the root archive at `archive` by its path without the
    /// leading `./`: its type, its link's target and its bytes.
    fn entries(archive: &Path) -> BTreeMap<String, (tar::EntryType, Option<PathBuf>, Vec<u8>)> {
        let decoder = zstd::stream::read::Decoder::new(std::fs::File::open(archive).unwrap()).unwrap();
        let mut archive = tar::Archive::new(decoder);
        let mut entries = BTreeMap::new();
        for entry in archive.entries().unwrap() {
            let mut entry = entry.unwrap();
            let path = entry.path().unwrap().to_string_lossy().into_owned();
            let path = path.trim_start_matches("./").trim_end_matches('/').to_owned();
            let kind = entry.header().entry_type();
            let link = entry.link_name().unwrap().map(|target| target.into_owned());
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).unwrap();
            entries.insert(path, (kind, link, bytes));
        }
        entries
    }

    #[tokio::test]
    async fn an_image_embeds_its_verified_inputs_and_publishes_a_manifest_the_manager_imports() {
        // The repository's pins are what a real build downloads.
        Pins::pinned().unwrap();
        let fixture = Fixture::new(DATABASE, b"commands").await;
        let client = demi_artifact::client_allowing_http().unwrap();
        let cancel = CancellationToken::new();
        let base = package(&fixture.options, ARCHITECTURE, &fixture.pins, &client, &cancel)
            .await
            .unwrap();
        let output = &fixture.options.output;
        let mut published: Vec<String> = std::fs::read_dir(output)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        published.sort();
        assert_eq!(published, [IMAGE_MANIFEST, RootfsFile::TarZst.name()]);
        let bytes = std::fs::read(output.join(IMAGE_MANIFEST)).unwrap();
        assert_eq!(base, measured(&bytes).await.sha256);
        let manifest = CloudImageManifest::decode(&bytes).unwrap();
        assert_eq!(manifest.architecture, ARCHITECTURE);
        assert_eq!(manifest.ubuntu, "26.04");
        let packages: Vec<(&str, &str)> = manifest
            .packages
            .iter()
            .map(|package| (package.name.as_str(), package.version.as_str()))
            .collect();
        assert_eq!(packages, [("base-files", "14ubuntu1"), ("tini", "0.19.0-3")]);
        assert_eq!(manifest.runner, fixture.runner);
        assert_eq!(manifest.releases, fixture.releases);
        let chrome = &fixture.pins.chrome.platforms[0];
        let uv = &fixture.pins.uv.arm64;
        let tools = [("uv", "0.12.13", uv.sha256.as_str()), ("chrome", "153.0.8010.36", chrome.sha256.as_str())];
        let recorded: Vec<(&str, &str, &str)> = manifest
            .tools
            .iter()
            .map(|tool| (tool.name.as_str(), tool.version.as_str(), tool.sha256.as_str()))
            .collect();
        assert_eq!(recorded, tools);
        let commands = measured(b"commands").await;
        let claude = measured(b"claude").await;
        let executables = BTreeMap::from([
            (format!("{ARTIFACTS_PATH}/{}/demi-claude", claude.sha256), claude.clone()),
            (format!("{ARTIFACTS_PATH}/{}/demi-commands", commands.sha256), commands.clone()),
            (format!("{IMAGE_BROWSERS}/{}/{CHROME}", chrome.sha256), measured(b"chrome").await),
            (RUNNER_PATH.to_owned(), measured(b"runner").await),
            (INIT_PATH.to_owned(), measured(b"tini").await),
            (format!("{TOOLS_PATH}/uv"), measured(b"uv").await),
            (format!("{TOOLS_PATH}/uvx"), measured(b"uvx").await),
        ]);
        assert_eq!(manifest.executables, executables);
        let archive = output.join(RootfsFile::TarZst.name());
        let rootfs = measured(&std::fs::read(&archive).unwrap()).await;
        assert_eq!((manifest.rootfs.sha256.as_str(), manifest.rootfs.size), (rootfs.sha256.as_str(), rootfs.size));
        // The archive holds every executable with the bytes the manifest
        // names, the `demi` alias, Chrome's receipt without its install lock,
        // and only the kinds of entry the manager's import accepts.
        let entries = entries(&archive);
        for (path, artifact) in &manifest.executables {
            let (kind, _, bytes) = &entries[path.trim_start_matches('/')];
            assert_eq!(*kind, tar::EntryType::Regular, "{path}");
            assert_eq!(measured(bytes).await, *artifact, "{path}");
        }
        let (kind, link, _) = &entries["usr/bin/demi"];
        assert_eq!((*kind, link.as_deref()), (tar::EntryType::Symlink, Some(Path::new("demi-runner"))));
        let browsers = IMAGE_BROWSERS.trim_start_matches('/');
        assert!(entries.contains_key(&format!("{browsers}/{}/receipt.json", chrome.sha256)));
        let paths: Vec<&String> = entries.keys().collect();
        assert!(!paths.iter().any(|path| path.ends_with(".lock")), "{paths:?}");
        let accepted = [
            tar::EntryType::Regular,
            tar::EntryType::Directory,
            tar::EntryType::Symlink,
            tar::EntryType::Link,
        ];
        assert!(entries.values().all(|(kind, ..)| accepted.contains(kind)), "{paths:?}");
    }

    #[tokio::test]
    async fn an_artifact_unlike_its_release_or_an_unfinished_package_fails_the_image_and_publishes_nothing() {
        let client = demi_artifact::client_allowing_http().unwrap();
        let cancel = CancellationToken::new();
        // demi-commands' file is not the executable its descriptor records.
        let corrupt = Fixture::new(DATABASE, b"COMMANDS").await;
        let refused = package(&corrupt.options, ARCHITECTURE, &corrupt.pins, &client, &cancel).await;
        assert!(
            matches!(&refused, Err(Error::File { path, source: demi_artifact::Error::Digest }) if path.ends_with("demi-commands")),
            "{refused:?}"
        );
        assert!(!corrupt.options.output.exists());
        // dpkg lists a package whose installation did not finish.
        let database = DATABASE.replace("Status: install ok installed\nArchitecture", "Status: install ok half-configured\nArchitecture");
        let unfinished = Fixture::new(&database, b"commands").await;
        let refused = package(&unfinished.options, ARCHITECTURE, &unfinished.pins, &client, &cancel).await;
        assert!(matches!(&refused, Err(Error::Unfinished { package, .. }) if package == "tini"), "{refused:?}");
        assert!(!unfinished.options.output.exists());
    }
}

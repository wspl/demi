//! `xtask cloud-image package`: the second stage of a Cloud image build, which
//! `cloud-guest-image/rootfs/build.sh` runs as root on a Linux builder of the
//! image's architecture, with an `xtask` built for that architecture.
//!
//! Into the Ubuntu tree the script made, it installs, from a server release and
//! its files:
//!
//! - The runner and its `demi` alias that the runner release names.
//! - Each command package's executable under its content-addressed path, each
//!   checked against its release's record.
//!
//! It then:
//!
//! - Reads the package inventory from the tree's dpkg database without running
//!   a program of the image: the packages dpkg records as installed, with their
//!   versions. A package whose installation did not finish fails the build, and
//!   one removed with only its configuration left is not installed.
//! - Takes the Ubuntu release from the tree's own os-release.
//! - Writes the root archive with GNU tar, which keeps numeric ownership, ACLs
//!   and extended attributes.
//! - Checks the manifest the way the machine manager decodes it.
//! - Publishes the release directory through `artifact`'s release publication
//!   and prints the base version.
//!
//! Everything of Demi's own lies under `/opt/demi`, which the machine manager
//! replaces with the configured image's at every boot, whatever image a Cloud
//! is pinned to:
//!
//! - `/opt/demi/bin/demi-runner` and its alias `/opt/demi/bin/demi`, with
//!   `/usr/bin/demi-runner` and `/usr/bin/demi` as links to them.
//! - Each command package's executable at
//!   `/opt/demi/artifacts/<sha256>/<executable>`, from which the runner starts
//!   command services instead of downloading them, after checking each against
//!   the backend's pinned descriptor.
//!
//! The image also makes the runner's two directories, private to `demi`:
//! `/var/lib/demi` and `/var/log/demi`. Nothing is compiled, pulled or
//! extracted on an ordinary wake, and a Docker daemon plays no part.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use demi_command_protocol::{PackageArtifact, PackageDescriptor};
use demi_machine_manager_protocol::image::{
    Architecture, CloudImageManifest, FormatVersion, INIT_PATH, InstalledPackage, ManifestError,
    Os, RUNNER_PATH, RootfsArchive, RootfsFile,
};
use demi_runner_protocol::image::ARTIFACTS_PATH;
use demi_runner_protocol::release::{RUNNER, RunnerRelease, compressed_file};
use demi_shared_artifacts::{
    Digest, Mode, Permissions, Publication, ReleaseFile, ReleaseRecord,
};
use tokio_util::sync::CancellationToken;

use crate::native::{DESCRIPTOR, MANIFEST};

/// The image release's record.
const IMAGE_MANIFEST: &str = "manifest.json";
/// The name the runner also answers to, beside it in `/opt/demi/bin`.
const RUNNER_ALIAS: &str = "demi";
/// Where the runner and its alias are linked from, on every `PATH`.
const LINKS_PATH: &str = "/usr/bin";
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
    /// The server release whose runner and command packages the image
    /// embeds: the runner its `runners/manifest.json` names, and every
    /// package in its `commands/`.
    #[arg(long, value_name = "DIRECTORY")]
    release: PathBuf,
    /// The directory of the release's files, which hold the programs.
    #[arg(long, value_name = "DIRECTORY")]
    files: PathBuf,
    /// The release directory to publish, a new one for every build.
    #[arg(long, value_name = "DIRECTORY")]
    output: PathBuf,
}

/// Why an image could not be packaged.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("a Cloud image is packaged on a Linux builder of its architecture")]
    NotLinux,
    #[error("{}: {source}", path.display())]
    File {
        path: PathBuf,
        source: demi_shared_artifacts::Error,
    },
    #[error("{} {reason}", path.display())]
    Invalid { path: PathBuf, reason: String },
    #[error("{release} carries nothing for {target}")]
    NotCarried {
        release: String,
        target: &'static str,
    },
    #[error("the command package {0} is released twice")]
    Twice(String),
    #[error("dpkg lists {package} as \"{status}\": its installation did not finish")]
    Unfinished { package: String, status: String },
    #[error("tar failed: {0}")]
    Tar(std::process::ExitStatus),
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    #[error(transparent)]
    Artifact(#[from] demi_shared_artifacts::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub fn run(command: Command) -> Result<(), Error> {
    let Command::Package(options) = command;
    // The builder's architecture is the image's, and xtask runs on it.
    let architecture = Architecture::host()
        .filter(|_| cfg!(target_os = "linux"))
        .ok_or(Error::NotLinux)?;
    let base = crate::interruptible(|cancel| async move {
        package(&options, architecture, &cancel).await
    })??;
    println!("{base}");
    Ok(())
}

/// Completes the image of the tree at `options.root` for `architecture`,
/// publishes it at `options.output` and returns its base version. The tree
/// and the release records are read before anything is installed, so a bad
/// input fails before the tree changes.
async fn package(
    options: &Options,
    architecture: Architecture,
    cancel: &CancellationToken,
) -> Result<String, Error> {
    let root = std::path::absolute(&options.root)?;
    let output = std::path::absolute(&options.output)?;
    let target = architecture.target();
    let packages = installed_packages(&root).await?;
    let ubuntu = ubuntu_release(&root).await?;
    let release = std::path::absolute(&options.release)?;
    let files = std::path::absolute(&options.files)?;
    let (runner, runner_artifact) = runner_release(&release.join("runners"), target).await?;
    let mut releases: Vec<(String, PackageDescriptor, PackageArtifact)> = Vec::new();
    for (executable, directory) in command_packages(&release.join("commands")).await? {
        let (descriptor, artifact) = package_release(&directory, target).await?;
        if releases
            .iter()
            .any(|(_, release, _)| release.id == descriptor.id)
        {
            return Err(Error::Twice(descriptor.id));
        }
        releases.push((executable, descriptor, artifact));
    }

    let mut executables = BTreeMap::new();
    install_runner(&root, &files, &runner, &runner_artifact, target).await?;
    executables.insert(RUNNER_PATH.to_owned(), runner_artifact);
    for (executable, descriptor, artifact) in &releases {
        let path = install_package(&root, &files, executable, descriptor, artifact, target).await?;
        executables.insert(path, artifact.clone());
    }
    executables.insert(
        INIT_PATH.to_owned(),
        measure(&in_tree(&root, INIT_PATH), cancel).await?,
    );

    let parent = output
        .parent()
        .expect("an absolute release directory has a parent");
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(at(parent))?;
    // The archive is written beside the release, then published into it.
    let written = tempfile::Builder::new()
        .prefix(".cloud-image-")
        .tempdir_in(parent)?;
    let archive = written.path().join(RootfsFile::TarZst.name());
    eprintln!("Cloud image: writing {}", RootfsFile::TarZst.name());
    let writing = {
        let root = root.clone();
        let archive = archive.clone();
        let cancel = cancel.clone();
        tokio::task::spawn_blocking(move || write_archive(&root, &archive, &cancel))
    };
    writing.await.map_err(std::io::Error::other)??;
    let digest = demi_shared_artifacts::digest(&archive, u64::MAX, cancel).await?;
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
        releases: releases
            .into_iter()
            .map(|(_, descriptor, _)| descriptor)
            .collect(),
        runner,
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
    demi_shared_artifacts::publish_release(&output, record, &files, cancel).await?;
    eprintln!("Cloud image: {}", output.display());
    // The base version names the manifest's bytes as published.
    let published =
        demi_shared_artifacts::digest(&output.join(IMAGE_MANIFEST), u64::MAX, cancel).await?;
    Ok(published.sha256)
}

/// The runner release the manifest of `runners` names, and its executable
/// for `target`.
async fn runner_release(
    runners: &Path,
    target: &'static str,
) -> Result<(RunnerRelease, PackageArtifact), Error> {
    let pointer = runners.join(MANIFEST);
    let bytes = tokio::fs::read(&pointer).await.map_err(at(&pointer))?;
    let release = RunnerRelease::decode(&bytes)
        .map_err(|error| invalid(&pointer, format!("is invalid: {error}")))?;
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
async fn package_release(
    directory: &Path,
    target: &'static str,
) -> Result<(PackageDescriptor, PackageArtifact), Error> {
    let path = directory.join(DESCRIPTOR);
    let bytes = tokio::fs::read(&path).await.map_err(at(&path))?;
    let value = serde_json::from_slice(&bytes)
        .map_err(|error| invalid(&path, format!("is invalid: {error}")))?;
    let descriptor = PackageDescriptor::parse(value)
        .map_err(|error| invalid(&path, format!("is invalid: {error}")))?;
    let artifact = descriptor
        .targets
        .get(target)
        .cloned()
        .ok_or_else(|| Error::NotCarried {
            release: format!(
                "the command package {}@{}",
                descriptor.id, descriptor.version
            ),
            target,
        })?;
    Ok((descriptor, artifact))
}

/// The command package releases in `commands`, a server release's
/// `commands/`: each directory's name, the program's, and the directory, in
/// the order of their names.
async fn command_packages(commands: &Path) -> Result<Vec<(String, PathBuf)>, Error> {
    let mut entries = tokio::fs::read_dir(commands).await.map_err(at(commands))?;
    let mut packages = Vec::new();
    while let Some(entry) = entries.next_entry().await.map_err(at(commands))? {
        let path = entry.path();
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| invalid(&path, "is not named in UTF-8"))?;
        packages.push((name, path));
    }
    packages.sort();
    Ok(packages)
}

/// Installs the runner of `release`, whose compressed copy is among `files`, as
/// `/opt/demi/bin/demi-runner` with `demi` as its alias beside it, and links
/// both from `/usr/bin`: everything of Demi's lies under `/opt/demi`, which a
/// Cloud takes from the configured image
/// (`cloud-guest-image/rootfs/packages.txt`).
async fn install_runner(
    root: &Path,
    files: &Path,
    release: &RunnerRelease,
    artifact: &PackageArtifact,
    target: &str,
) -> Result<(), Error> {
    let source = files.join(compressed_file(RUNNER, target));
    let installed = in_tree(root, RUNNER_PATH);
    let directory = installed.parent().expect("the runner's path has a directory");
    tokio::fs::create_dir_all(directory)
        .await
        .map_err(at(directory))?;
    install_decoded(&source, artifact, &installed).await?;
    // `demi` is the runner by another name, beside it.
    let name = installed
        .file_name()
        .expect("the runner's path names a file");
    let alias = installed.with_file_name(RUNNER_ALIAS);
    symlink(name, &alias).await?;
    let links = in_tree(root, LINKS_PATH);
    tokio::fs::create_dir_all(&links).await.map_err(at(&links))?;
    for program in [Path::new(RUNNER_PATH), &Path::new(RUNNER_PATH).with_file_name(RUNNER_ALIAS)] {
        let name = program.file_name().expect("a program's path names a file");
        symlink(program.as_os_str(), &links.join(name)).await?;
    }
    eprintln!("Cloud image: runner release {}", release.release);
    Ok(())
}

/// Installs `executable`, the program of `descriptor`'s release, whose
/// compressed copy is among `files`, under its content-addressed path, and
/// returns that path in the image. Nothing is there unless the copy decodes
/// to `artifact`.
async fn install_package(
    root: &Path,
    files: &Path,
    executable: &str,
    descriptor: &PackageDescriptor,
    artifact: &PackageArtifact,
    target: &str,
) -> Result<String, Error> {
    let source = files.join(compressed_file(executable, target));
    let path = format!("{ARTIFACTS_PATH}/{}/{executable}", artifact.sha256);
    let installed = in_tree(root, &path);
    let parent = installed
        .parent()
        .expect("an artifact's path has a directory");
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(at(parent))?;
    install_decoded(&source, artifact, &installed).await?;
    eprintln!(
        "Cloud image: command package {}@{}",
        descriptor.id, descriptor.version
    );
    Ok(path)
}

/// Installs the executable whose compressed copy is at `source` as
/// `destination`: nothing is there unless the copy decodes to `artifact`.
async fn install_decoded(
    source: &Path,
    artifact: &PackageArtifact,
    destination: &Path,
) -> Result<(), Error> {
    let encoded = tokio::fs::read(source).await.map_err(at(source))?;
    let expected = Digest {
        size: artifact.size,
        sha256: artifact.sha256.clone(),
    };
    let decoded = tokio::task::spawn_blocking(move || {
        demi_shared_artifacts::decode_blocking(&encoded, &expected)
    })
    .await
    .map_err(std::io::Error::other)?
    .map_err(|error| Error::File {
        path: source.to_owned(),
        source: error,
    })?;
    let publication = Publication {
        mode: Mode::CreateNew,
        permissions: Permissions::Executable,
        durable: false,
    };
    demi_shared_artifacts::publish_bytes(destination, &decoded, publication)
        .await
        .map_err(at(destination))
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

/// The size and SHA-256 of the file at `path` in the tree, which must be a
/// regular file rather than a link, which could lead out of the tree.
async fn measure(path: &Path, cancel: &CancellationToken) -> Result<PackageArtifact, Error> {
    let metadata = tokio::fs::symlink_metadata(path).await.map_err(at(path))?;
    if !metadata.is_file() {
        return Err(invalid(path, "is not a regular file"));
    }
    let digest = demi_shared_artifacts::digest(path, u64::MAX, cancel)
        .await
        .map_err(at(path))?;
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
        _ => {
            return Err(invalid(
                database,
                "lists a package without its name or status",
            ));
        }
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
fn compress(
    mut input: impl std::io::Read,
    archive: &Path,
    cancel: &CancellationToken,
) -> Result<(), Error> {
    let file = std::fs::File::create_new(archive).map_err(at(archive))?;
    // Level 0 is zstd's default level.
    let mut encoder = zstd::stream::write::Encoder::new(file, 0)?;
    let mut buffer = vec![0; 1024 * 1024];
    loop {
        if cancel.is_cancelled() {
            return Err(demi_shared_artifacts::Error::Cancelled.into());
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
fn at<E: Into<demi_shared_artifacts::Error>>(path: &Path) -> impl FnOnce(E) -> Error {
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

//! `cargo xtask native package` (`builds-and-releases.md` § Packaging): the
//! built executables of the named targets become a release directory,
//! published through `artifact`'s verified release publication. A command
//! package's descriptor lists the operations its contract crate declares; a
//! runner release is named by the SHA-256 of its versions and targets, and
//! the top-level manifest names the release packaged last; the backend's and
//! the machine manager's record names the executable, its version and its
//! targets. A command package carries each executable's zstd-compressed copy
//! beside it, kept by the executable's SHA-256 so that packaging an unchanged
//! program again compresses nothing, and is versioned with the workspace
//! version only when published by the release workflow, with a development
//! version naming its artifacts otherwise (`package-versioning.md` § Rust
//! executables).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use demi_command_protocol::{PackageArtifact, PackageDescriptor, canonical_digest};
use demi_runner_protocol::release::{
    RUNNER, RunnerRelease, SERVER_RELEASE, ServerRelease, compressed_file, release_file,
};
use demi_shared_artifacts::{Mode, Permissions, Publication, ReleaseFile, ReleaseRecord};
use serde::Serialize;
use tokio_util::sync::CancellationToken;

use super::{DESCRIPTOR, Error, Executable, MANIFEST};

/// The workspace version, which every crate inherits: the version a command
/// package, the backend and the machine manager are released as.
const VERSION: &str = env!("CARGO_PKG_VERSION");
/// A backend or machine manager release's record.
const RELEASE: &str = "release.json";
/// Where packaging keeps the executables' compressed copies, each named by
/// the executable's SHA-256, so packaging again compresses nothing.
const COMPRESSED: &str = ".cache/compressed";
/// What a compressed copy's name adds to its executable's.
const COMPRESSED_SUFFIX: &str = ".zst";
/// How many hexadecimal digits of its artifacts' digest a development
/// version carries.
const DEVELOPMENT_DIGITS: usize = 12;

#[derive(clap::Args)]
pub struct Options {
    /// The executable to package.
    #[arg(long = "package", value_name = "CRATE")]
    package: Executable,
    /// A target to package; repeat for several [default: every target of the
    /// executable].
    #[arg(long = "target", value_name = "TRIPLE", value_parser = super::target)]
    targets: Vec<&'static str>,
    /// The Cargo target directory the build wrote [default:
    /// .cache/native-target in the repository].
    #[arg(long, value_name = "DIRECTORY")]
    artifacts: Option<PathBuf>,
    /// The release directory; for the runner, the directory of its releases.
    #[arg(long, value_name = "DIRECTORY")]
    output: PathBuf,
    #[command(flatten)]
    caches: Caches,
}

/// Where packaging keeps what it would otherwise make again.
#[derive(clap::Args, Clone)]
pub struct Caches {
    /// Where the executables' compressed copies are kept, each named by the
    /// executable's SHA-256 [default: .cache/compressed in the repository].
    #[arg(long, value_name = "DIRECTORY")]
    pub compressed: Option<PathBuf>,
}

impl Caches {
    /// The directory `named`, or `default` in the repository.
    fn directory(named: Option<&Path>, default: &str) -> Result<PathBuf, Error> {
        Ok(match named {
            Some(directory) => std::path::absolute(directory)?,
            None => crate::repository().join(default),
        })
    }
}

/// How a command package is versioned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Versioning {
    /// The workspace version itself: only the release workflow's packaging.
    Published,
    /// The workspace version with build metadata naming its artifacts.
    Development,
}

/// One release to package: the executable, the targets it carries, the
/// Cargo target directory the build wrote, and the release directory.
pub struct Spec<'a> {
    pub executable: Executable,
    pub targets: &'a [&'static str],
    pub artifacts: &'a Path,
    pub output: &'a Path,
    pub caches: &'a Caches,
    pub versioning: Versioning,
}

pub fn run(options: Options) -> Result<(), Error> {
    let packaged = crate::interruptible(|cancel| async move {
        let targets = super::targets(&[options.package], &options.targets)?;
        let artifacts = super::artifacts(options.artifacts.as_deref())?;
        let output = std::path::absolute(&options.output)?;
        let spec = Spec {
            executable: options.package,
            targets: &targets,
            artifacts: &artifacts,
            output: &output,
            caches: &options.caches,
            versioning: Versioning::Development,
        };
        package(&spec, &cancel).await
    })??;
    println!("{packaged}");
    Ok(())
}

/// The kind of release an executable has.
enum Release {
    Runner,
    /// A command package: its id, and the operations its program serves, as
    /// its contract crate declares them.
    Package {
        id: &'static str,
        operations: Vec<String>,
    },
    /// The backend or the machine manager: the executable of one version.
    Executable,
}

/// What a release carries: the built executable of each target.
struct Built {
    files: Vec<ReleaseFile>,
    targets: BTreeMap<String, PackageArtifact>,
}

impl Release {
    fn of(executable: Executable) -> Result<Self, Error> {
        Ok(match executable {
            Executable::Runner => Self::Runner,
            Executable::File => Self::Package {
                id: demi_command_package_file_protocol::PACKAGE,
                operations: demi_command_package_file_protocol::OPERATIONS
                    .iter()
                    .map(|&name| name.to_owned())
                    .collect(),
            },
            Executable::Browser => Self::Package {
                id: demi_command_package_browser_protocol::PACKAGE,
                operations: demi_command_package_browser_protocol::Operation::names()
                    .map(String::from)
                    .collect(),
            },
            Executable::Claude => Self::Package {
                id: demi_command_package_claude_code_protocol::PACKAGE,
                operations: demi_command_package_claude_code_protocol::Operation::ALL
                    .map(|operation| operation.name().to_owned())
                    .to_vec(),
            },
            Executable::Backend | Executable::Machines => Self::Executable,
        })
    }
}

impl Built {
    fn new() -> Self {
        Self {
            files: Vec::new(),
            targets: BTreeMap::new(),
        }
    }

    /// Adds `source`, the build of `executable` for `target`.
    async fn add(
        &mut self,
        executable: Executable,
        target: &'static str,
        source: PathBuf,
        cancel: &CancellationToken,
    ) -> Result<(), Error> {
        let digest = match demi_shared_artifacts::digest(&source, u64::MAX, cancel).await {
            Ok(digest) => digest,
            Err(demi_shared_artifacts::Error::Io(error))
                if error.kind() == std::io::ErrorKind::NotFound =>
            {
                return Err(Error::NotBuilt {
                    executable,
                    target,
                    path: source,
                });
            }
            Err(error) => return Err(error.into()),
        };
        self.targets.insert(
            target.to_owned(),
            PackageArtifact {
                sha256: digest.sha256.clone(),
                size: digest.size,
            },
        );
        self.files.push(ReleaseFile {
            source,
            path: Path::new(target).join(executable.file_name(target)),
            digest,
            executable: true,
        });
        Ok(())
    }

    /// Adds the compressed copy of each executable the release carries,
    /// beside it, taken from `cache`, where a copy missing is made first.
    async fn add_compressed(&mut self, cache: &Path, cancel: &CancellationToken) -> Result<(), Error> {
        let executables: Vec<ReleaseFile> = self
            .files
            .iter()
            .filter(|file| file.executable)
            .cloned()
            .collect();
        for executable in executables {
            let source = cache.join(&executable.digest.sha256);
            if !tokio::fs::try_exists(&source).await? {
                compress(&executable.source, &source).await?;
            }
            let digest = demi_shared_artifacts::digest(&source, u64::MAX, cancel).await?;
            let mut name = executable.path.clone().into_os_string();
            name.push(COMPRESSED_SUFFIX);
            self.files.push(ReleaseFile {
                source,
                path: name.into(),
                digest,
                executable: false,
            });
        }
        Ok(())
    }

    /// The version of the command package this release is, as `versioning`
    /// names it: a development version carries the leading digits of the
    /// digest of its targets' executables.
    fn version(&self, versioning: Versioning) -> Result<String, Error> {
        match versioning {
            Versioning::Published => Ok(VERSION.to_owned()),
            Versioning::Development => {
                let digest = canonical_digest(&self.targets)
                    .map_err(|error| Error::Record(error.to_string()))?;
                Ok(format!("{VERSION}+dev.{}", &digest[..DEVELOPMENT_DIGITS]))
            }
        }
    }
}

/// Writes `executable` compressed for its download to `destination`, which
/// holds the copy only once it is whole.
async fn compress(executable: &Path, destination: &Path) -> Result<(), Error> {
    if let Some(parent) = destination.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    eprintln!("Compressing {}", executable.display());
    let bytes = tokio::fs::read(executable).await?;
    let encoded = tokio::task::spawn_blocking(move || {
        demi_shared_artifacts::encode_blocking(&bytes, demi_shared_artifacts::Effort::Published)
    })
    .await
    .map_err(std::io::Error::other)??;
    let publication = Publication {
        mode: Mode::Replace,
        permissions: Permissions::Default,
        durable: true,
    };
    demi_shared_artifacts::publish_bytes(destination, &encoded, publication).await?;
    Ok(())
}

/// Takes the release of `executable` packaged at `release` apart into a
/// server release (`builds-and-releases.md` § Server release): its records
/// into the root at `root`, the runner's manifests into `runners/` and a
/// command package's descriptor into `commands/<program>/`, and its
/// programs into `files`, each runner executable as it is and each command
/// program as its compressed copy. A file already in `files` must hold the
/// same bytes.
pub async fn split(
    release: &Path,
    executable: Executable,
    root: &Path,
    files: &Path,
    cancel: &CancellationToken,
) -> Result<(), Error> {
    tokio::fs::create_dir_all(files).await?;
    if executable == Executable::Runner {
        let bytes = tokio::fs::read(release.join(MANIFEST)).await?;
        let manifest =
            RunnerRelease::decode(&bytes).map_err(|error| Error::Record(error.to_string()))?;
        let runners = root.join("runners");
        let current = runners.join(&manifest.release);
        tokio::fs::create_dir_all(&current).await?;
        tokio::fs::write(runners.join(MANIFEST), &bytes).await?;
        tokio::fs::write(current.join(MANIFEST), &bytes).await?;
        for target in manifest.targets.keys() {
            let built = release
                .join(&manifest.release)
                .join(target)
                .join(executable.file_name(target));
            place(&built, &files.join(release_file(RUNNER, target)), cancel).await?;
        }
        return Ok(());
    }
    let bytes = tokio::fs::read(release.join(DESCRIPTOR)).await?;
    let value = serde_json::from_slice(&bytes).map_err(|error| Error::Record(error.to_string()))?;
    let descriptor =
        PackageDescriptor::parse(value).map_err(|error| Error::Record(error.to_string()))?;
    let package = root.join("commands").join(executable.name());
    tokio::fs::create_dir_all(&package).await?;
    tokio::fs::write(package.join(DESCRIPTOR), &bytes).await?;
    for target in descriptor.targets.keys() {
        let mut copy = executable.file_name(target);
        copy.push_str(COMPRESSED_SUFFIX);
        let file = files.join(compressed_file(executable.name(), target));
        place(&release.join(target).join(copy), &file, cancel).await?;
    }
    Ok(())
}

/// Writes `record` as the `release.json` of the server release root at
/// `root`.
pub async fn write_server_release(root: &Path, record: &ServerRelease) -> Result<(), Error> {
    tokio::fs::write(root.join(SERVER_RELEASE), self::record(record)?).await?;
    Ok(())
}

/// Puts a copy of `source` at `file`, unless `file` holds the same bytes
/// already; a file of other bytes there is refused.
async fn place(source: &Path, file: &Path, cancel: &CancellationToken) -> Result<(), Error> {
    if tokio::fs::try_exists(file).await? {
        let wanted = demi_shared_artifacts::digest(source, u64::MAX, cancel).await?;
        let found = demi_shared_artifacts::digest(file, u64::MAX, cancel).await?;
        if found != wanted {
            return Err(Error::OtherFile(file.to_owned()));
        }
        return Ok(());
    }
    let publication = Publication {
        mode: Mode::CreateNew,
        permissions: Permissions::Default,
        durable: true,
    };
    let mut input = tokio::fs::File::open(source).await?;
    demi_shared_artifacts::publish(file, &mut input, publication, cancel).await?;
    Ok(())
}

/// Publishes the release `spec` names and says what it published.
pub async fn package(spec: &Spec<'_>, cancel: &CancellationToken) -> Result<String, Error> {
    let executable = spec.executable;
    let mut built = Built::new();
    for &target in spec.targets {
        let source = super::built(spec.artifacts, executable, target);
        built.add(executable, target, source, cancel).await?;
    }
    match Release::of(executable)? {
        Release::Runner => runner(spec.output, built, cancel).await,
        Release::Package { id, operations } => {
            let compressed = Caches::directory(spec.caches.compressed.as_deref(), COMPRESSED)?;
            built.add_compressed(&compressed, cancel).await?;
            let version = built.version(spec.versioning)?;
            command_package(spec.output, id, &version, operations, built, cancel).await
        }
        Release::Executable => executable_release(spec.output, executable, built, cancel).await,
    }
}

/// Publishes at `output` the development release of `command`, one of
/// [`Executable::COMMANDS`], whose one target is this machine's and whose
/// program is `program` (`backend.md` § One-command development backend).
#[cfg(all(unix, feature = "developer"))]
pub async fn development_package(
    command: Executable,
    program: PathBuf,
    output: &Path,
    cancel: &CancellationToken,
) -> Result<(), Error> {
    let Release::Package { id, operations } = Release::of(command)?
    else {
        panic!("{} is not a command program", command.name());
    };
    let mut built = Built::new();
    built
        .add(
            command,
            demi_command_protocol::host_target(),
            program,
            cancel,
        )
        .await?;
    built
        .add_compressed(&crate::repository().join(COMPRESSED), cancel)
        .await?;
    let version = built.version(Versioning::Development)?;
    command_package(output, id, &version, operations, built, cancel).await?;
    Ok(())
}

/// Publishes the command package `id` of `version`, which serves
/// `operations`, at `output`.
async fn command_package(
    output: &Path,
    id: &str,
    version: &str,
    operations: Vec<String>,
    built: Built,
    cancel: &CancellationToken,
) -> Result<String, Error> {
    let descriptor = PackageDescriptor {
        id: id.to_owned(),
        version: version.to_owned(),
        protocol_version: demi_command_protocol::VERSION,
        operations,
        targets: built.targets,
    };
    let digest = descriptor
        .digest()
        .map_err(|error| Error::Record(error.to_string()))?;
    let bytes = record(&descriptor)?;
    let record = ReleaseRecord {
        name: DESCRIPTOR,
        bytes: &bytes,
    };
    demi_shared_artifacts::publish_release(output, record, &built.files, cancel).await?;
    Ok(format!(
        "Command package {id}@{version}: {digest}\n{}",
        output.join(DESCRIPTOR).display()
    ))
}

/// A backend or machine manager release's record: the executable, the
/// workspace version it carries, and each target's file.
#[derive(Serialize)]
struct ExecutableRelease<'a> {
    executable: &'a str,
    version: &'a str,
    targets: &'a BTreeMap<String, PackageArtifact>,
}

/// Publishes the release of the backend or the machine manager at `output`.
async fn executable_release(
    output: &Path,
    executable: Executable,
    built: Built,
    cancel: &CancellationToken,
) -> Result<String, Error> {
    let bytes = record(&ExecutableRelease {
        executable: executable.name(),
        version: VERSION,
        targets: &built.targets,
    })?;
    let record = ReleaseRecord {
        name: RELEASE,
        bytes: &bytes,
    };
    demi_shared_artifacts::publish_release(output, record, &built.files, cancel).await?;
    Ok(format!(
        "Release {}@{VERSION}\n{}",
        executable.name(),
        output.join(RELEASE).display()
    ))
}

/// What a runner release's identity is the SHA-256 of: its versions and
/// targets, the release record without its identity.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RunnerContents<'a> {
    wire: u32,
    command_protocol: u64,
    targets: &'a BTreeMap<String, PackageArtifact>,
}

/// Publishes a runner release in its own directory of `output`, then points
/// `output`'s manifest at it.
async fn runner(output: &Path, built: Built, cancel: &CancellationToken) -> Result<String, Error> {
    let wire = demi_runner_protocol::wire::VERSION;
    let command_protocol = demi_command_protocol::VERSION;
    let contents = RunnerContents {
        wire,
        command_protocol,
        targets: &built.targets,
    };
    let release = RunnerRelease {
        release: canonical_digest(&contents).map_err(|error| Error::Record(error.to_string()))?,
        wire,
        command_protocol,
        targets: built.targets,
    };
    release
        .validate()
        .map_err(|error| Error::Record(error.to_string()))?;
    let bytes = record(&release)?;
    let directory = output.join(&release.release);
    let manifest = ReleaseRecord {
        name: MANIFEST,
        bytes: &bytes,
    };
    demi_shared_artifacts::publish_release(&directory, manifest, &built.files, cancel).await?;
    // The pointer moves only once the release it names is in place.
    let pointer = Publication {
        mode: Mode::Replace,
        permissions: Permissions::Default,
        durable: true,
    };
    demi_shared_artifacts::publish_bytes(&output.join(MANIFEST), &bytes, pointer).await?;
    Ok(format!("Runner release: {}", directory.display()))
}

/// A release record's file, as `crate::record` writes it.
fn record(value: &impl Serialize) -> Result<Vec<u8>, Error> {
    crate::record(value).map_err(|error| Error::Record(error.to_string()))
}

#[cfg(test)]
mod tests {
    use demi_command_protocol::TARGETS;

    use super::*;

    /// A Cargo target directory with a fixture executable of `executable`
    /// for each of `targets`, whose bytes are `contents` and the target.
    fn build(artifacts: &Path, executable: Executable, targets: &[&str], contents: &str) {
        for target in targets {
            let directory = artifacts.join(target).join("release");
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(
                directory.join(executable.file_name(target)),
                format!("{contents} {target}"),
            )
            .unwrap();
        }
    }

    /// Packages `executable`'s build in `artifacts` for `targets` (every
    /// target of the executable when none is named) at `output`, with the
    /// caches beside the build.
    async fn package_at(
        executable: Executable,
        artifacts: &Path,
        output: &Path,
        targets: &[&'static str],
        cancel: &CancellationToken,
    ) -> Result<String, Error> {
        let targets = super::super::targets(&[executable], targets)?;
        let caches = Caches {
            compressed: Some(artifacts.join("compressed")),
        };
        let spec = Spec {
            executable,
            targets: &targets,
            artifacts,
            output,
            caches: &caches,
            versioning: Versioning::Development,
        };
        package(&spec, cancel).await
    }

    fn names(directory: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn pointer(output: &Path) -> RunnerRelease {
        RunnerRelease::decode(&std::fs::read(output.join(MANIFEST)).unwrap()).unwrap()
    }

    #[tokio::test]
    async fn each_runner_release_has_its_directory_and_the_manifest_names_the_last_one_in_place() {
        let root = tempfile::tempdir().unwrap();
        let artifacts = root.path().join("artifacts");
        let output = root.path().join("runners");
        let cancel = CancellationToken::new();
        build(&artifacts, Executable::Runner, TARGETS, "first");
        package_at(Executable::Runner, &artifacts, &output, &[], &cancel)
        .await
        .unwrap();
        let first = pointer(&output);
        assert_eq!(first.targets.len(), TARGETS.len());
        assert_eq!(
            std::fs::read(output.join(&first.release).join(MANIFEST)).unwrap(),
            std::fs::read(output.join(MANIFEST)).unwrap()
        );
        let windows = output
            .join(&first.release)
            .join("x86_64-pc-windows-msvc/demi-runner.exe");
        assert_eq!(
            std::fs::read(windows).unwrap(),
            b"first x86_64-pc-windows-msvc"
        );
        // Packaged again, the same release is the one in place.
        package_at(Executable::Runner, &artifacts, &output, &[], &cancel)
        .await
        .unwrap();
        assert_eq!(pointer(&output), first);
        // Another build is another release, which the manifest names from
        // now on; the first one stays for the runners installed from it.
        build(&artifacts, Executable::Runner, TARGETS, "second");
        package_at(Executable::Runner, &artifacts, &output, &[], &cancel)
        .await
        .unwrap();
        let second = pointer(&output);
        assert_ne!(second.release, first.release);
        let mut releases = vec![
            first.release.clone(),
            second.release.clone(),
            MANIFEST.to_owned(),
        ];
        releases.sort();
        assert_eq!(names(&output), releases);
        // A release whose bytes changed in place is refused, and the
        // manifest keeps naming the release it named.
        let linux = output
            .join(&first.release)
            .join("x86_64-unknown-linux-musl/demi-runner");
        std::fs::write(&linux, b"corrupt").unwrap();
        build(&artifacts, Executable::Runner, TARGETS, "first");
        let refused = package_at(Executable::Runner, &artifacts, &output, &[], &cancel)
        .await;
        assert!(
            matches!(&refused, Err(Error::Artifact(demi_shared_artifacts::Error::Conflict(path))) if *path == linux),
            "{refused:?}"
        );
        assert_eq!(pointer(&output), second);
        assert_eq!(names(&output), releases);
    }

    #[tokio::test]
    async fn a_backend_or_manager_release_records_its_version_and_is_immutable_per_version() {
        let root = tempfile::tempdir().unwrap();
        let artifacts = root.path().join("artifacts");
        let linux = ["aarch64-unknown-linux-musl", "x86_64-unknown-linux-musl"];
        let cancel = CancellationToken::new();
        build(&artifacts, Executable::Machines, &linux, "manager");
        let output = root.path().join("demi-machine-manager");
        package_at(Executable::Machines, &artifacts, &output, &[], &cancel)
        .await
        .unwrap();
        let mut targets = serde_json::Map::new();
        for target in linux {
            let path = artifacts.join(target).join("release/demi-machine-manager");
            let digest = demi_shared_artifacts::digest(&path, u64::MAX, &cancel)
                .await
                .unwrap();
            targets.insert(
                target.to_owned(),
                serde_json::json!({"sha256": digest.sha256, "size": digest.size}),
            );
        }
        let record: serde_json::Value =
            serde_json::from_slice(&std::fs::read(output.join(RELEASE)).unwrap()).unwrap();
        let expected = serde_json::json!({"executable": "demi-machine-manager", "version": VERSION, "targets": targets});
        assert_eq!(record, expected);
        assert_eq!(names(&output), [linux[0], RELEASE, linux[1]]);
        assert_eq!(
            std::fs::read(output.join(linux[1]).join("demi-machine-manager")).unwrap(),
            b"manager x86_64-unknown-linux-musl"
        );
        // Another build of the same version is refused: a published version
        // is immutable.
        build(&artifacts, Executable::Machines, &linux, "rebuilt");
        let refused = package_at(Executable::Machines, &artifacts, &output, &[], &cancel)
        .await;
        assert!(
            matches!(
                refused,
                Err(Error::Artifact(demi_shared_artifacts::Error::Conflict(_)))
            ),
            "{refused:?}"
        );
    }

    #[tokio::test]
    async fn a_development_release_carries_the_named_targets_and_its_programs_operations() {
        let root = tempfile::tempdir().unwrap();
        let artifacts = root.path().join("artifacts");
        let carried = ["aarch64-apple-darwin", "x86_64-unknown-linux-musl"];
        let cancel = CancellationToken::new();
        build(&artifacts, Executable::File, &carried, "file");
        build(&artifacts, Executable::Runner, &carried, "runner");
        // Without named targets, a release needs every target's build.
        let output = root.path().join("demi-file");
        let incomplete = package_at(Executable::File, &artifacts, &output, &[], &cancel)
        .await;
        assert!(
            matches!(incomplete, Err(Error::NotBuilt { .. })),
            "{incomplete:?}"
        );
        assert!(!output.exists());
        package_at(Executable::File, &artifacts, &output, &carried, &cancel)
        .await
        .unwrap();
        let descriptor =
            serde_json::from_slice(&std::fs::read(output.join(DESCRIPTOR)).unwrap()).unwrap();
        let descriptor = PackageDescriptor::parse(descriptor).unwrap();
        assert_eq!(descriptor.id, demi_command_package_file_protocol::PACKAGE);
        assert_eq!(
            descriptor.operations,
            demi_command_package_file_protocol::OPERATIONS
        );
        assert_eq!(descriptor.targets.keys().collect::<Vec<_>>(), carried);
        assert_eq!(names(&output), [carried[0], DESCRIPTOR, carried[1]]);
        // Beside each executable lies its compressed copy, which decodes to
        // it.
        let linux = output.join(carried[1]);
        assert_eq!(names(&linux), ["demi-file", "demi-file.zst"]);
        let decoded = zstd::decode_all(&std::fs::read(linux.join("demi-file.zst")).unwrap()[..]);
        assert_eq!(decoded.unwrap(), std::fs::read(linux.join("demi-file")).unwrap());
        // A development version names the build: the same programs give the
        // same version, a rebuilt one another.
        let development = descriptor.version.clone();
        assert!(
            development.starts_with(&format!("{VERSION}+dev.")),
            "{development}"
        );
        let again = root.path().join("demi-file-again");
        package_at(Executable::File, &artifacts, &again, &carried, &cancel)
            .await
            .unwrap();
        let version = |output: &Path| {
            let value =
                serde_json::from_slice(&std::fs::read(output.join(DESCRIPTOR)).unwrap()).unwrap();
            PackageDescriptor::parse(value).unwrap().version
        };
        assert_eq!(version(&again), development);
        build(&artifacts, Executable::File, &carried, "rebuilt");
        let rebuilt = root.path().join("demi-file-rebuilt");
        package_at(Executable::File, &artifacts, &rebuilt, &carried, &cancel)
            .await
            .unwrap();
        assert_ne!(version(&rebuilt), development);
        let runners = root.path().join("runners");
        package_at(Executable::Runner, &artifacts, &runners, &carried, &cancel)
        .await
        .unwrap();
        let release = pointer(&runners);
        assert_eq!(release.targets.keys().collect::<Vec<_>>(), carried);
        assert_eq!(
            names(&runners.join(&release.release)),
            [carried[0], MANIFEST, carried[1]]
        );
    }
}

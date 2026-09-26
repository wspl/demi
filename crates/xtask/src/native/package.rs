//! `cargo xtask native package` (`builds-and-releases.md` § Packaging): the
//! built executables of the named targets become a release directory,
//! published through `artifact`'s verified release publication. A command
//! package's descriptor lists the operations its contract crate declares; a
//! runner release is named by the SHA-256 of its versions and targets, and
//! the top-level manifest names the release packaged last; the backend's and
//! the machine manager's record names the executable, its version and its
//! targets.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use demi_artifact::{Mode, Permissions, Publication, ReleaseFile, ReleaseRecord};
use demi_command_service::protocol::{PackageArtifact, PackageDescriptor, canonical_digest};
use demi_runner_protocol::release::RunnerRelease;
use serde::Serialize;
use tokio_util::sync::CancellationToken;

use super::{Error, Executable};

/// The workspace version, which every crate inherits: the version a command
/// package, the backend and the machine manager are released as.
const VERSION: &str = env!("CARGO_PKG_VERSION");
/// A command package release's record.
const DESCRIPTOR: &str = "descriptor.json";
/// A runner release's record, and the pointer beside the releases.
const MANIFEST: &str = "manifest.json";
/// A backend or machine manager release's record.
const RELEASE: &str = "release.json";

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
}

pub fn run(options: Options) -> Result<(), Error> {
    let packaged = crate::interruptible(|cancel| async move { package(&options, &cancel).await })??;
    println!("{packaged}");
    Ok(())
}

/// The kind of release an executable has.
enum Release {
    Runner,
    /// A command package: its id and the operations its program serves, as
    /// its contract crate declares them.
    Package { id: &'static str, operations: Vec<String> },
    /// The backend or the machine manager: the executable of one version.
    Executable,
}

/// What a release carries of its executable: the built file of each target.
struct Built {
    files: Vec<ReleaseFile>,
    targets: BTreeMap<String, PackageArtifact>,
}

/// Publishes the release `options` name and says what it published.
async fn package(options: &Options, cancel: &CancellationToken) -> Result<String, Error> {
    let executable = options.package;
    let release = match executable {
        Executable::Runner => Release::Runner,
        Executable::Commands => Release::Package {
            id: demi_builtin_protocol::PACKAGE,
            operations: demi_builtin_protocol::Operation::names().map(String::from).collect(),
        },
        Executable::Claude => Release::Package {
            id: demi_claude_protocol::PACKAGE,
            operations: demi_claude_protocol::Operation::ALL
                .map(|operation| operation.name().to_owned())
                .to_vec(),
        },
        Executable::Backend | Executable::Machines => Release::Executable,
    };
    let artifacts = super::artifacts(options.artifacts.as_deref())?;
    let output = std::path::absolute(&options.output)?;
    let mut built = Built {
        files: Vec::new(),
        targets: BTreeMap::new(),
    };
    for target in super::targets(&[executable], &options.targets)? {
        let name = executable.file_name(target);
        let source = artifacts.join(target).join("release").join(&name);
        let digest = match demi_artifact::digest(&source, u64::MAX, cancel).await {
            Ok(digest) => digest,
            Err(demi_artifact::Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(Error::NotBuilt {
                    executable,
                    target,
                    path: source,
                });
            }
            Err(error) => return Err(error.into()),
        };
        built.targets.insert(
            target.to_owned(),
            PackageArtifact {
                sha256: digest.sha256.clone(),
                size: digest.size,
            },
        );
        built.files.push(ReleaseFile {
            source,
            path: Path::new(target).join(name),
            digest,
            executable: true,
        });
    }
    match release {
        Release::Runner => runner(&output, built, cancel).await,
        Release::Package { id, operations } => command_package(&output, id, operations, built, cancel).await,
        Release::Executable => executable_release(&output, executable, built, cancel).await,
    }
}

/// Publishes the command package `id`, which serves `operations`, at
/// `output`.
async fn command_package(
    output: &Path,
    id: &str,
    operations: Vec<String>,
    built: Built,
    cancel: &CancellationToken,
) -> Result<String, Error> {
    let descriptor = PackageDescriptor {
        id: id.to_owned(),
        version: VERSION.to_owned(),
        protocol_version: demi_command_service::protocol::VERSION,
        operations,
        targets: built.targets,
    };
    let digest = descriptor.digest().map_err(|error| Error::Record(error.to_string()))?;
    let bytes = record(&descriptor)?;
    let record = ReleaseRecord {
        name: DESCRIPTOR,
        bytes: &bytes,
    };
    demi_artifact::publish_release(output, record, &built.files, cancel).await?;
    Ok(format!(
        "Native package {id}@{VERSION}: {digest}\n{}",
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
    demi_artifact::publish_release(output, record, &built.files, cancel).await?;
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
    let command_protocol = demi_command_service::protocol::VERSION;
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
    release.validate().map_err(|error| Error::Record(error.to_string()))?;
    let bytes = record(&release)?;
    let directory = output.join(&release.release);
    let manifest = ReleaseRecord {
        name: MANIFEST,
        bytes: &bytes,
    };
    demi_artifact::publish_release(&directory, manifest, &built.files, cancel).await?;
    // The pointer moves only once the release it names is in place.
    let pointer = Publication {
        mode: Mode::Replace,
        permissions: Permissions::Default,
        durable: true,
    };
    demi_artifact::publish_bytes(&output.join(MANIFEST), &bytes, pointer).await?;
    Ok(format!("Runner release: {}", directory.display()))
}

/// A release record's file: indented JSON with a final newline.
fn record(value: &impl Serialize) -> Result<Vec<u8>, Error> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(|error| Error::Record(error.to_string()))?;
    bytes.push(b'\n');
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use demi_command_service::protocol::TARGETS;

    use super::*;

    /// A Cargo target directory with a fixture executable of `executable`
    /// for each of `targets`, whose bytes are `contents` and the target.
    fn build(artifacts: &Path, executable: Executable, targets: &[&str], contents: &str) {
        for target in targets {
            let directory = artifacts.join(target).join("release");
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(directory.join(executable.file_name(target)), format!("{contents} {target}")).unwrap();
        }
    }

    fn options(executable: Executable, artifacts: &Path, output: &Path, targets: &[&'static str]) -> Options {
        Options {
            package: executable,
            targets: targets.to_vec(),
            artifacts: Some(artifacts.to_owned()),
            output: output.to_owned(),
        }
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
        package(&options(Executable::Runner, &artifacts, &output, &[]), &cancel).await.unwrap();
        let first = pointer(&output);
        assert_eq!(first.targets.len(), TARGETS.len());
        assert_eq!(
            std::fs::read(output.join(&first.release).join(MANIFEST)).unwrap(),
            std::fs::read(output.join(MANIFEST)).unwrap()
        );
        let windows = output.join(&first.release).join("x86_64-pc-windows-msvc/demi-runner.exe");
        assert_eq!(std::fs::read(windows).unwrap(), b"first x86_64-pc-windows-msvc");
        // Packaged again, the same release is the one in place.
        package(&options(Executable::Runner, &artifacts, &output, &[]), &cancel).await.unwrap();
        assert_eq!(pointer(&output), first);
        // Another build is another release, which the manifest names from
        // now on; the first one stays for the runners installed from it.
        build(&artifacts, Executable::Runner, TARGETS, "second");
        package(&options(Executable::Runner, &artifacts, &output, &[]), &cancel).await.unwrap();
        let second = pointer(&output);
        assert_ne!(second.release, first.release);
        let mut releases = vec![first.release.clone(), second.release.clone(), MANIFEST.to_owned()];
        releases.sort();
        assert_eq!(names(&output), releases);
        // A release whose bytes changed in place is refused, and the
        // manifest keeps naming the release it named.
        let linux = output.join(&first.release).join("x86_64-unknown-linux-musl/demi-runner");
        std::fs::write(&linux, b"corrupt").unwrap();
        build(&artifacts, Executable::Runner, TARGETS, "first");
        let refused = package(&options(Executable::Runner, &artifacts, &output, &[]), &cancel).await;
        assert!(
            matches!(&refused, Err(Error::Artifact(demi_artifact::Error::Conflict(path))) if *path == linux),
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
        let output = root.path().join("demi-machines");
        package(&options(Executable::Machines, &artifacts, &output, &[]), &cancel)
            .await
            .unwrap();
        let mut targets = serde_json::Map::new();
        for target in linux {
            let path = artifacts.join(target).join("release/demi-machines");
            let digest = demi_artifact::digest(&path, u64::MAX, &cancel).await.unwrap();
            targets.insert(target.to_owned(), serde_json::json!({"sha256": digest.sha256, "size": digest.size}));
        }
        let record: serde_json::Value = serde_json::from_slice(&std::fs::read(output.join(RELEASE)).unwrap()).unwrap();
        let expected = serde_json::json!({"executable": "demi-machines", "version": VERSION, "targets": targets});
        assert_eq!(record, expected);
        assert_eq!(names(&output), [linux[0], RELEASE, linux[1]]);
        assert_eq!(
            std::fs::read(output.join(linux[1]).join("demi-machines")).unwrap(),
            b"manager x86_64-unknown-linux-musl"
        );
        // Another build of the same version is refused: a published version
        // is immutable.
        build(&artifacts, Executable::Machines, &linux, "rebuilt");
        let refused = package(&options(Executable::Machines, &artifacts, &output, &[]), &cancel).await;
        assert!(matches!(refused, Err(Error::Artifact(demi_artifact::Error::Conflict(_)))), "{refused:?}");
    }

    #[tokio::test]
    async fn a_development_release_carries_the_named_targets_and_its_programs_operations() {
        let root = tempfile::tempdir().unwrap();
        let artifacts = root.path().join("artifacts");
        let carried = ["aarch64-apple-darwin", "x86_64-unknown-linux-musl"];
        let cancel = CancellationToken::new();
        build(&artifacts, Executable::Commands, &carried, "commands");
        build(&artifacts, Executable::Runner, &carried, "runner");
        // Without named targets, a release needs every target's build.
        let output = root.path().join("demi-builtin");
        let incomplete = package(&options(Executable::Commands, &artifacts, &output, &[]), &cancel).await;
        assert!(matches!(incomplete, Err(Error::NotBuilt { .. })), "{incomplete:?}");
        assert!(!output.exists());
        package(&options(Executable::Commands, &artifacts, &output, &carried), &cancel)
            .await
            .unwrap();
        let descriptor = serde_json::from_slice(&std::fs::read(output.join(DESCRIPTOR)).unwrap()).unwrap();
        let descriptor = PackageDescriptor::parse(descriptor).unwrap();
        assert_eq!(descriptor.id, demi_builtin_protocol::PACKAGE);
        assert_eq!(descriptor.version, VERSION);
        let declared: Vec<&str> = demi_builtin_protocol::Operation::names().collect();
        assert_eq!(descriptor.operations, declared);
        assert_eq!(descriptor.targets.keys().collect::<Vec<_>>(), carried);
        assert_eq!(names(&output), [carried[0], DESCRIPTOR, carried[1]]);
        let runners = root.path().join("runners");
        package(&options(Executable::Runner, &artifacts, &runners, &carried), &cancel)
            .await
            .unwrap();
        let release = pointer(&runners);
        assert_eq!(release.targets.keys().collect::<Vec<_>>(), carried);
        assert_eq!(names(&runners.join(&release.release)), [carried[0], MANIFEST, carried[1]]);
    }
}

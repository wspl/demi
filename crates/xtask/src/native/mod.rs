//! `xtask native` (`builds-and-releases.md`): builds the workspace's
//! executables for their targets with this machine's toolchain and cross
//! tools, and packages the built executables into releases.

mod build;
mod package;

#[cfg(all(unix, feature = "developer"))]
pub use package::development_package;
pub use package::{Caches, Spec, Versioning, package, split, write_server_release};

use std::path::{Path, PathBuf};

use demi_command_protocol::TARGETS;

/// Where the builds go and the packaging reads them, unless `--artifacts`
/// names another Cargo target directory.
const ARTIFACTS: &str = ".cache/native-target";
/// A command package release's record.
pub const DESCRIPTOR: &str = "descriptor.json";
/// A runner release's record, and the pointer beside the releases.
pub const MANIFEST: &str = "manifest.json";

#[derive(clap::Subcommand)]
pub enum Command {
    /// Compiles executables for their targets into a Cargo target directory.
    Build(build::Options),
    /// Turns built executables into a release directory.
    Package(package::Options),
}

pub fn run(command: Command) -> Result<(), Error> {
    match command {
        Command::Build(options) => build::run(options),
        Command::Package(options) => package::run(options),
    }
}

/// Why a build or a packaging stopped.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{} is not built for {target}", executable.name())]
    NotItsTarget {
        executable: Executable,
        target: &'static str,
    },
    #[error(
        "the Apple targets need the Apple SDK {}: pass --sdk or set SDKROOT",
        build::APPLE_SDK_VERSION
    )]
    NoSdk,
    #[error("the Apple SDK at {} is {found}, not the pinned {}", path.display(), build::APPLE_SDK_VERSION)]
    SdkVersion { path: PathBuf, found: String },
    #[error("{}: {reason}", path.display())]
    SdkSettings { path: PathBuf, reason: String },
    #[error(
        "a Windows build runs in a Visual Studio developer environment: VCToolsVersion and WindowsSDKVersion are not set"
    )]
    NoMsvc,
    #[error("Visual Studio selects MSVC {toolset} and Windows SDK {sdk}, not the pinned {pinned}")]
    MsvcVersion {
        toolset: String,
        sdk: String,
        pinned: String,
    },
    #[error("the build for {target} failed: {status}")]
    Build {
        target: &'static str,
        status: std::process::ExitStatus,
    },
    #[error("no build of {} for {target} at {}: run bun xtask native build first", executable.name(), path.display())]
    NotBuilt {
        executable: Executable,
        target: &'static str,
        path: PathBuf,
    },
    #[error("the release record is invalid: {0}")]
    Record(String),
    #[error("{} holds another file of that name: the files directory belongs to another build", .0.display())]
    OtherFile(PathBuf),
    #[error(transparent)]
    Artifact(#[from] demi_shared_artifacts::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// An executable of the workspace, named by its Cargo package
/// (`builds-and-releases.md` § Executables and targets).
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Executable {
    #[value(name = "demi-runner")]
    Runner,
    #[value(name = "demi-file")]
    File,
    #[value(name = "demi-browser")]
    Browser,
    #[value(name = "demi-claude-code")]
    Claude,
    #[value(name = "demi-backend")]
    Backend,
    #[value(name = "demi-machine-manager")]
    Machines,
    #[value(name = "demi-server")]
    Server,
}

impl Executable {
    /// The executables a build makes unless named: the runner and the
    /// command programs.
    const DEFAULT: [Self; 4] = [Self::Runner, Self::File, Self::Browser, Self::Claude];
    /// The command programs, each the program of a command package.
    pub const COMMANDS: [Self; 3] = [Self::File, Self::Browser, Self::Claude];

    /// The Cargo package, which is also the executable's name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Runner => demi_runner_protocol::release::RUNNER,
            Self::File => "demi-file",
            Self::Browser => "demi-browser",
            Self::Claude => "demi-claude-code",
            Self::Backend => "demi-backend",
            Self::Machines => "demi-machine-manager",
            Self::Server => "demi-server",
        }
    }

    /// The targets it runs on, in the order of the release matrix.
    fn targets(self) -> &'static [&'static str] {
        match self {
            Self::Runner | Self::File | Self::Browser | Self::Claude => TARGETS,
            // Servers run Linux; a developer may also run the backend on a
            // Mac, with the Cloud in a Lima VM.
            Self::Backend => &[
                "aarch64-apple-darwin",
                "x86_64-apple-darwin",
                "aarch64-unknown-linux-musl",
                "x86_64-unknown-linux-musl",
            ],
            // gVisor, namespaces, cgroups, loop devices and nftables exist
            // only on Linux, and so do the servers demi-server installs.
            Self::Machines | Self::Server => {
                &["aarch64-unknown-linux-musl", "x86_64-unknown-linux-musl"]
            }
        }
    }

    /// The executable's file name on `target`.
    pub(crate) fn file_name(self, target: &str) -> String {
        let suffix = if windows(target) { ".exe" } else { "" };
        format!("{}{suffix}", self.name())
    }
}

/// Parses a `--target` option: one of the release matrix's targets.
pub fn target(value: &str) -> Result<&'static str, String> {
    TARGETS
        .iter()
        .copied()
        .find(|target| *target == value)
        .ok_or_else(|| format!("the targets are {}", TARGETS.join(", ")))
}

fn windows(target: &str) -> bool {
    target.contains("windows")
}

fn apple(target: &str) -> bool {
    target.contains("apple")
}

/// The targets `executables` are built or packaged for, in the release
/// matrix's order: the `named` ones, each of which every executable must
/// run on, or else every target of each executable.
pub fn targets(
    executables: &[Executable],
    named: &[&'static str],
) -> Result<Vec<&'static str>, Error> {
    for target in named {
        if let Some(executable) = executables
            .iter()
            .find(|executable| !executable.targets().contains(target))
        {
            return Err(Error::NotItsTarget {
                executable: *executable,
                target,
            });
        }
    }
    let selected = |target: &&str| {
        if named.is_empty() {
            executables
                .iter()
                .any(|executable| executable.targets().contains(target))
        } else {
            named.contains(target)
        }
    };
    Ok(TARGETS.iter().copied().filter(selected).collect())
}

/// Where a build of `executable` for `target` lies in the Cargo target
/// directory `artifacts`.
pub fn built(artifacts: &Path, executable: Executable, target: &str) -> PathBuf {
    artifacts
        .join(target)
        .join("release")
        .join(executable.file_name(target))
}

/// The Cargo target directory `artifacts` names, or the default one.
pub fn artifacts(named: Option<&Path>) -> Result<PathBuf, Error> {
    let directory = match named {
        Some(directory) => std::path::absolute(directory)?,
        None => crate::repository().join(ARTIFACTS),
    };
    Ok(directory)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_targets_must_be_each_executables_and_unnamed_ones_are_all_of_theirs() {
        let refused = targets(
            &[Executable::Runner, Executable::Machines],
            &["aarch64-apple-darwin"],
        );
        assert!(
            matches!(
                refused,
                Err(Error::NotItsTarget {
                    executable: Executable::Machines,
                    target: "aarch64-apple-darwin"
                })
            ),
            "{refused:?}"
        );
        let linux = ["x86_64-unknown-linux-musl", "aarch64-unknown-linux-musl"];
        assert_eq!(
            targets(&[Executable::Runner, Executable::Machines], &linux).unwrap(),
            ["aarch64-unknown-linux-musl", "x86_64-unknown-linux-musl"]
        );
        assert_eq!(
            targets(&[Executable::Machines], &[]).unwrap(),
            Executable::Machines.targets()
        );
        assert_eq!(targets(&Executable::DEFAULT, &[]).unwrap(), TARGETS);
    }
}

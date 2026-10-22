//! `xtask server-release`: assembles a server release from the executables
//! `xtask native build` wrote.
//!
//! A server release is one directory per version and Linux target, its root, in
//! which the backend and the machine manager find everything they serve,
//! publish or run, with the release's files beside it. For example, 0.1.3 for
//! x86_64:
//!
//! ```text
//! /opt/demi/releases/0.1.3/
//!   bin/demi-backend              the target's backend
//!   bin/demi-machine-manager      the target's machine manager
//!   bin/demi-server               the server's installer and upgrader
//!   systemd/                      the units of the backend and the manager
//!   web/                          the built web app, with its build.json
//!   runners/                      the runner release's manifests
//!   commands/demi-file/           one command package release per directory,
//!   commands/demi-browser/        its descriptor
//!   ...
//!   release.json                  where the release's files are
//!   image/                        the Cloud image release of the architecture
//!
//! the release's files:
//!   demi-runner-x86_64-unknown-linux-musl.zst   a runner executable's
//!   demi-runner-x86_64-pc-windows-msvc.exe.zst  compressed copy, per target
//!   demi-file-x86_64-unknown-linux-musl.zst     a command program's
//!   ...                                         compressed copy, per target
//! ```
//!
//! `release.json` names where the files are: for a published release, the
//! GitHub release that offers them; for a developer's, the directory they lie
//! in. The backend takes each program from there the first time a Host needs
//! it, so a server downloads the Windows programs only when a Windows laptop
//! pairs.
//!
//! What reads the root:
//!
//! - The backend serves `web/`, installs runners from `runners/` and publishes
//!   every package under `commands/`.
//! - The machine manager imports `image/`, built from the same release with its
//!   Linux target's programs taken from the release's files, so the command
//!   artifacts it embeds are the ones the backend's catalog selects and a Cloud
//!   starts them from the image instead of downloading them.
//!
//! Parts of the root are optional:
//!
//! - A root without `--server` has no `bin/` or `systemd/` (a developer's
//!   backend runs from the Cargo target directory).
//! - One without `--web` has no web app (Vite serves it in development).
//! - One without `image/` serves a backend whose machine manager runs
//!   elsewhere.
//!
//! No root carries gVisor.
//!
//! Its `LICENSE`, `LICENSE-DEMI` and `NOTICE` are the repository's, and with a
//! web app its `THIRD-PARTY-NOTICES` are the ones `xtask notices` wrote into
//! it. Packaging makes each release whole first, so its records and its files
//! are the ones packaging checked.
//!
//! The root is assembled in a stage beside it and renamed into place, so it
//! exists whole or not at all, and is never changed in place. The files
//! directory may already hold this build's files, as when the build's other
//! Linux root was assembled into it, and a file in place must then be the same.

use std::path::{Path, PathBuf};

use demi_runner_protocol::release::ServerRelease;
use tokio_util::sync::CancellationToken;

use crate::native::{self, Caches, Executable, Spec, Versioning};
use crate::notices::WEB_NOTICES;

/// The file a built web app holds, which the backend reads its build from.
const WEB_BUILD: &str = "build.json";

/// The project's licenses, the Apache License and the Demi License that
/// covers the server and the web app, and its notice, which every release
/// carries at its root.
const LEGAL: [(&str, &str); 3] = [
    ("LICENSE", include_str!("../../../LICENSE")),
    ("LICENSE-DEMI", include_str!("../../../LICENSE-DEMI")),
    ("NOTICE", include_str!("../../../NOTICE")),
];

/// The third-party notices at a release's root.
const NOTICES: &str = "THIRD-PARTY-NOTICES";

/// The services' units, which each program's crate keeps and a server
/// release carries (`crates/server/src/main.rs`).
const UNITS: [(&str, &str); 2] = [
    (
        "demi-backend.service",
        include_str!("../../backend/systemd/demi-backend.service"),
    ),
    (
        "demi-machine-manager.service",
        include_str!("../../machine-manager/systemd/demi-machine-manager.service"),
    ),
];

#[derive(clap::Args)]
pub struct Options {
    /// The root to assemble, a directory that does not exist yet.
    #[arg(long, value_name = "DIRECTORY")]
    pub(crate) output: PathBuf,
    /// The directory of the release's files, which may hold this build's
    /// files already.
    #[arg(long, value_name = "DIRECTORY")]
    pub(crate) files: PathBuf,
    /// The HTTPS URL the files will be published at, which release.json
    /// names [default: the files directory].
    #[arg(long, value_name = "URL")]
    pub(crate) downloads: Option<String>,
    /// A target of the runner and the command packages; repeat for several
    /// [default: all six].
    #[arg(long = "target", value_name = "TRIPLE", value_parser = native::target)]
    pub(crate) targets: Vec<&'static str>,
    /// The Linux target whose backend and machine manager go in bin/, with
    /// the units in systemd/ [default: no bin/].
    #[arg(long, value_name = "TRIPLE", value_parser = native::target)]
    pub(crate) server: Option<&'static str>,
    /// The built web app to copy into web/ [default: no web/].
    #[arg(long, value_name = "DIRECTORY")]
    pub(crate) web: Option<PathBuf>,
    /// Names the command packages with the workspace version itself; only
    /// the release workflow publishes.
    #[arg(long)]
    pub(crate) publish: bool,
    /// The Cargo target directory the build wrote [default:
    /// .cache/native-target in the repository].
    #[arg(long, value_name = "DIRECTORY")]
    pub(crate) artifacts: Option<PathBuf>,
    #[command(flatten)]
    pub(crate) caches: Caches,
}

/// Why assembling stopped.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{} exists: a server release root is assembled once", .0.display())]
    Exists(PathBuf),
    #[error("{0} is not a Linux target: a server runs Linux")]
    NotLinux(&'static str),
    #[error("{} holds no {WEB_BUILD}: it is not a built web app", .0.display())]
    NotWeb(PathBuf),
    #[error("{} holds no {WEB_NOTICES}: `xtask notices --web` writes them", .0.display())]
    NoNotices(PathBuf),
    #[error("{} has no parent directory to assemble it beside", .0.display())]
    NoParent(PathBuf),
    #[error("the release's record is invalid: {0}")]
    Record(String),
    #[error("copying the web app failed: {0}")]
    Copy(#[from] fs_extra::error::Error),
    #[error(transparent)]
    Native(#[from] native::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub fn run(options: Options) -> Result<(), Error> {
    let assembled = crate::interruptible(|cancel| async move { assemble(&options, &cancel).await })??;
    println!("Server release: {}", assembled.display());
    Ok(())
}

/// Assembles the root and the files `options` name and returns the root's
/// path.
pub(crate) async fn assemble(options: &Options, cancel: &CancellationToken) -> Result<PathBuf, Error> {
    let output = std::path::absolute(&options.output)?;
    if tokio::fs::try_exists(&output).await? {
        return Err(Error::Exists(output));
    }
    if let Some(server) = options.server
        && !server.contains("linux")
    {
        return Err(Error::NotLinux(server));
    }
    let web = match &options.web {
        Some(web) => {
            let web = std::path::absolute(web)?;
            if !tokio::fs::try_exists(web.join(WEB_BUILD)).await? {
                return Err(Error::NotWeb(web));
            }
            if !tokio::fs::try_exists(web.join(WEB_NOTICES)).await? {
                return Err(Error::NoNotices(web));
            }
            Some(web)
        }
        None => None,
    };
    let files = std::path::absolute(&options.files)?;
    let record = ServerRelease {
        files: match &options.downloads {
            Some(url) => url.clone(),
            None => files.to_string_lossy().into_owned(),
        },
    };
    garde::Validate::validate(&record).map_err(|report| Error::Record(report.to_string()))?;
    let parent = output
        .parent()
        .ok_or_else(|| Error::NoParent(output.clone()))?
        .to_owned();
    tokio::fs::create_dir_all(&parent).await?;
    tokio::fs::create_dir_all(&files).await?;
    let name = output
        .file_name()
        .ok_or_else(|| Error::NoParent(output.clone()))?
        .to_string_lossy()
        .into_owned();
    // The stages are removed however assembling ends, unless the root's
    // becomes the root.
    let stage = staged(&parent, format!(".{name}-stage-")).await?;
    let packaged = staged(&parent, format!(".{name}-packages-")).await?;
    let artifacts = native::artifacts(options.artifacts.as_deref())?;
    let versioning = if options.publish {
        Versioning::Published
    } else {
        Versioning::Development
    };
    for executable in [Executable::Runner].into_iter().chain(Executable::COMMANDS) {
        let targets = native::targets(&[executable], &options.targets)?;
        let release = packaged.path().join(executable.name());
        let spec = Spec {
            executable,
            targets: &targets,
            artifacts: &artifacts,
            output: &release,
            caches: &options.caches,
            versioning,
            version: demi_shared_artifacts::WORKSPACE_VERSION,
        };
        println!("{}", native::package(&spec, cancel).await?);
        native::split(&release, executable, stage.path(), &files, cancel).await?;
    }
    native::write_server_release(stage.path(), &record).await?;
    for (name, text) in LEGAL {
        tokio::fs::write(stage.path().join(name), text).await?;
    }
    if let Some(server) = options.server {
        let bin = stage.path().join("bin");
        tokio::fs::create_dir_all(&bin).await?;
        for executable in [Executable::Backend, Executable::Machines, Executable::Server] {
            let built = native::built(&artifacts, executable, server);
            if !tokio::fs::try_exists(&built).await? {
                return Err(native::Error::NotBuilt {
                    executable,
                    target: server,
                    path: built,
                }
                .into());
            }
            // `copy` keeps the executable's permissions.
            tokio::fs::copy(&built, bin.join(executable.name())).await?;
        }
        let systemd = stage.path().join("systemd");
        tokio::fs::create_dir_all(&systemd).await?;
        for (name, unit) in UNITS {
            tokio::fs::write(systemd.join(name), unit).await?;
        }
    }
    if let Some(web) = web {
        tokio::fs::copy(web.join(WEB_NOTICES), stage.path().join(NOTICES)).await?;
        copy_directory(web, stage.path().join("web")).await?;
    }
    if cancel.is_cancelled() {
        return Err(native::Error::Artifact(demi_shared_artifacts::Error::Cancelled).into());
    }
    let staged = stage.keep();
    if let Err(error) = tokio::fs::rename(&staged, &output).await {
        // Nothing else holds the stage: it goes rather than lingering.
        let _ = tokio::fs::remove_dir_all(&staged).await;
        return Err(error.into());
    }
    Ok(output)
}

/// Copies the contents of `source` into the new directory `destination`,
/// keeping each file's permissions.
async fn copy_directory(source: PathBuf, destination: PathBuf) -> Result<(), Error> {
    tokio::task::spawn_blocking(move || -> Result<(), Error> {
        std::fs::create_dir(&destination)?;
        let contents = fs_extra::dir::CopyOptions::new().content_only(true);
        fs_extra::dir::copy(&source, &destination, &contents)?;
        Ok(())
    })
    .await
    .map_err(std::io::Error::other)?
}

/// A new directory in `parent` whose name starts with `prefix`, removed
/// when dropped.
async fn staged(parent: &Path, prefix: String) -> Result<tempfile::TempDir, Error> {
    let parent = parent.to_owned();
    let directory = tokio::task::spawn_blocking(move || {
        tempfile::Builder::new().prefix(&prefix).tempdir_in(parent)
    })
    .await
    .map_err(std::io::Error::other)??;
    Ok(directory)
}

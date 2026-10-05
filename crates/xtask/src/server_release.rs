//! `cargo xtask server-release` (`builds-and-releases.md` § Server release):
//! assembles a server release from the executables `cargo xtask native
//! build` wrote. The root holds the runner release's manifests in
//! `runners/`, each command package's descriptor in `commands/`, the record
//! `release.json`, which says where the release's files are, and, when
//! asked, a Linux target's backend, machine manager and `demi-server` in
//! `bin/` with the
//! services' units in `systemd/`, and the built web app in `web/`. The release's files hold each target's runner
//! executable and each command program's compressed copy. Packaging makes
//! each release whole first, so its records and its files are the ones
//! packaging checked. The root is assembled in a stage beside it and renamed
//! into place, so it exists whole or not at all, and is never changed in
//! place; the files directory may already hold this build's files, as when
//! the build's other Linux root was assembled into it, and a file in place
//! must then be the same.

use std::path::{Path, PathBuf};

use demi_runner_protocol::release::ServerRelease;
use tokio_util::sync::CancellationToken;

use crate::native::{self, Caches, Executable, Spec, Versioning};

/// The file a built web app holds, which the backend reads its build from
/// (`web-api.md` § Serving the web app build).
const WEB_BUILD: &str = "build.json";

/// The services' units, which each program's crate keeps and a server
/// release carries (`upgrades.md` § One release on a server).
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
    output: PathBuf,
    /// The directory of the release's files, which may hold this build's
    /// files already.
    #[arg(long, value_name = "DIRECTORY")]
    files: PathBuf,
    /// The HTTPS URL the files will be published at, which release.json
    /// names [default: the files directory].
    #[arg(long, value_name = "URL")]
    downloads: Option<String>,
    /// A target of the runner and the command packages; repeat for several
    /// [default: all six].
    #[arg(long = "target", value_name = "TRIPLE", value_parser = native::target)]
    targets: Vec<&'static str>,
    /// The Linux target whose backend and machine manager go in bin/, with
    /// the units in systemd/ [default: no bin/].
    #[arg(long, value_name = "TRIPLE", value_parser = native::target)]
    server: Option<&'static str>,
    /// The built web app to copy into web/ [default: no web/].
    #[arg(long, value_name = "DIRECTORY")]
    web: Option<PathBuf>,
    /// Names the command packages with the workspace version itself; only
    /// the release workflow publishes.
    #[arg(long)]
    publish: bool,
    /// The Cargo target directory the build wrote [default:
    /// .cache/native-target in the repository].
    #[arg(long, value_name = "DIRECTORY")]
    artifacts: Option<PathBuf>,
    #[command(flatten)]
    caches: Caches,
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
async fn assemble(options: &Options, cancel: &CancellationToken) -> Result<PathBuf, Error> {
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
        };
        println!("{}", native::package(&spec, cancel).await?);
        native::split(&release, executable, stage.path(), &files, cancel).await?;
    }
    native::write_server_release(stage.path(), &record).await?;
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

#[cfg(test)]
mod tests {
    use std::path::Path;

    use demi_runner_protocol::release::{RunnerRelease, SERVER_RELEASE};

    use crate::native::{DESCRIPTOR, MANIFEST};

    use super::*;

    /// A Cargo target directory with a fixture build of `executable` for
    /// each of `targets`.
    fn build(artifacts: &Path, executables: &[Executable], targets: &[&str]) {
        for executable in executables {
            for target in targets {
                let built = native::built(artifacts, *executable, target);
                std::fs::create_dir_all(built.parent().unwrap()).unwrap();
                std::fs::write(&built, format!("{} {target}", executable.name())).unwrap();
            }
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

    #[tokio::test]
    async fn a_root_holds_the_records_and_its_files_the_programs_once() {
        let root = tempfile::tempdir().unwrap();
        let artifacts = root.path().join("artifacts");
        let linux = "x86_64-unknown-linux-musl";
        let windows = "aarch64-pc-windows-msvc";
        let mut programs = vec![Executable::Runner];
        programs.extend(Executable::COMMANDS);
        build(&artifacts, &programs, &[windows]);
        let web = root.path().join("dist");
        std::fs::create_dir_all(web.join("assets")).unwrap();
        std::fs::write(web.join(WEB_BUILD), r#"{"build":"fixture"}"#).unwrap();
        std::fs::write(web.join("assets/index.js"), "page").unwrap();
        let output = root.path().join("release");
        let files = root.path().join("files");
        let options = |output: &Path, downloads: Option<&str>| Options {
            output: output.to_owned(),
            files: files.clone(),
            downloads: downloads.map(str::to_owned),
            targets: vec![windows],
            server: Some(linux),
            web: Some(web.clone()),
            publish: false,
            artifacts: Some(artifacts.clone()),
            caches: Caches {
                compressed: Some(artifacts.join("compressed")),
            },
        };
        let cancel = CancellationToken::new();
        // Without the server's build, no root is assembled and no stage is
        // left behind.
        let missing = assemble(&options(&output, None), &cancel).await;
        assert!(
            matches!(missing, Err(Error::Native(native::Error::NotBuilt { .. }))),
            "{missing:?}"
        );
        assert_eq!(names(root.path()), ["artifacts", "dist", "files"]);
        build(
            &artifacts,
            &[Executable::Backend, Executable::Machines, Executable::Server],
            &[linux],
        );
        assemble(&options(&output, None), &cancel).await.unwrap();
        assert_eq!(
            names(&output),
            ["bin", "commands", "release.json", "runners", "systemd", "web"]
        );
        assert_eq!(
            names(&output.join("systemd")),
            ["demi-backend.service", "demi-machine-manager.service"]
        );
        assert_eq!(
            names(&output.join("bin")),
            ["demi-backend", "demi-machine-manager", "demi-server"]
        );
        assert_eq!(
            std::fs::read(output.join("bin/demi-backend")).unwrap(),
            format!("demi-backend {linux}").as_bytes()
        );
        // The root holds the records; the programs are the release's files.
        assert_eq!(
            names(&output.join("commands")),
            ["demi-browser", "demi-claude-code", "demi-file"]
        );
        assert_eq!(names(&output.join("commands/demi-file")), [DESCRIPTOR]);
        let runners = output.join("runners");
        let manifest = RunnerRelease::decode(&std::fs::read(runners.join(MANIFEST)).unwrap()).unwrap();
        assert_eq!(names(&runners.join(&manifest.release)), [MANIFEST]);
        assert_eq!(
            names(&files),
            [
                "demi-browser-aarch64-pc-windows-msvc.exe.zst",
                "demi-claude-code-aarch64-pc-windows-msvc.exe.zst",
                "demi-file-aarch64-pc-windows-msvc.exe.zst",
                "demi-runner-aarch64-pc-windows-msvc.exe",
            ]
        );
        assert_eq!(
            std::fs::read(files.join("demi-runner-aarch64-pc-windows-msvc.exe")).unwrap(),
            format!("demi-runner {windows}").as_bytes()
        );
        let record = |output: &Path| {
            ServerRelease::decode(&std::fs::read(output.join(SERVER_RELEASE)).unwrap()).unwrap()
        };
        assert_eq!(record(&output).files, files.to_str().unwrap());
        assert_eq!(names(&output.join("web")), ["assets", WEB_BUILD]);
        // A root is assembled once.
        let again = assemble(&options(&output, None), &cancel).await;
        assert!(matches!(again, Err(Error::Exists(_))), "{again:?}");
        // Another root of the same build shares the files, and names where
        // they will be published.
        let published = root.path().join("published");
        let downloads = "https://github.com/wspl/demi/releases/download/v0.1.3/";
        assemble(&options(&published, Some(downloads)), &cancel).await.unwrap();
        assert_eq!(record(&published).files, downloads);
        assert_eq!(names(&files).len(), 4);
        // A rebuilt program is another build's, whose files these are not.
        build(&artifacts, &[Executable::File], &[windows]);
        std::fs::write(
            native::built(&artifacts, Executable::File, windows),
            "demi-file rebuilt",
        )
        .unwrap();
        let other = assemble(&options(&root.path().join("other"), None), &cancel).await;
        assert!(
            matches!(other, Err(Error::Native(native::Error::OtherFile(_)))),
            "{other:?}"
        );
    }
}

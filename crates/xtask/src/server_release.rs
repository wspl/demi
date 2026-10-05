//! `cargo xtask server-release` (`builds-and-releases.md` § Server release):
//! assembles a server release root from the executables
//! `cargo xtask native build` wrote: the runner release in `runners/`, a
//! release of each command package in `commands/`, and, when asked, a Linux
//! target's backend and machine manager in `bin/` and the built web app in
//! `web/`. The root is assembled in a stage beside it and renamed into
//! place, so it exists whole or not at all, and is never changed in place.

use std::path::PathBuf;

use tokio_util::sync::CancellationToken;

use crate::native::{self, Caches, Executable, Spec, Versioning};

/// The file a built web app holds, which the backend reads its build from
/// (`web-api.md` § Serving the web app build).
const WEB_BUILD: &str = "build.json";

#[derive(clap::Args)]
pub struct Options {
    /// The root to assemble, a directory that does not exist yet.
    #[arg(long, value_name = "DIRECTORY")]
    output: PathBuf,
    /// A target of runners/ and commands/; repeat for several [default: all
    /// six].
    #[arg(long = "target", value_name = "TRIPLE", value_parser = native::target)]
    targets: Vec<&'static str>,
    /// The Linux target whose backend and machine manager go in bin/
    /// [default: no bin/].
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

/// Assembles the root `options` name and returns its path.
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
    let parent = output
        .parent()
        .ok_or_else(|| Error::NoParent(output.clone()))?
        .to_owned();
    tokio::fs::create_dir_all(&parent).await?;
    let name = output
        .file_name()
        .ok_or_else(|| Error::NoParent(output.clone()))?
        .to_string_lossy()
        .into_owned();
    // The stage is removed however assembling ends, unless it becomes the
    // root.
    let stage = tokio::task::spawn_blocking(move || {
        tempfile::Builder::new()
            .prefix(&format!(".{name}-stage-"))
            .tempdir_in(parent)
    })
    .await
    .map_err(std::io::Error::other)??;
    let artifacts = native::artifacts(options.artifacts.as_deref())?;
    let versioning = if options.publish {
        Versioning::Published
    } else {
        Versioning::Development
    };
    let releases = [Executable::Runner]
        .into_iter()
        .chain(Executable::COMMANDS)
        .map(|executable| {
            let directory = match executable {
                Executable::Runner => stage.path().join("runners"),
                command => stage.path().join("commands").join(command.name()),
            };
            (executable, directory)
        });
    for (executable, directory) in releases {
        let targets = native::targets(&[executable], &options.targets)?;
        let spec = Spec {
            executable,
            targets: &targets,
            artifacts: &artifacts,
            output: &directory,
            caches: &options.caches,
            versioning,
        };
        println!("{}", native::package(&spec, cancel).await?);
    }
    if let Some(server) = options.server {
        let bin = stage.path().join("bin");
        tokio::fs::create_dir_all(&bin).await?;
        for executable in [Executable::Backend, Executable::Machines] {
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
    }
    if let Some(web) = web {
        let destination = stage.path().join("web");
        tokio::task::spawn_blocking(move || -> Result<(), Error> {
            std::fs::create_dir(&destination)?;
            let contents = fs_extra::dir::CopyOptions::new().content_only(true);
            fs_extra::dir::copy(&web, &destination, &contents)?;
            Ok(())
        })
        .await
        .map_err(std::io::Error::other)??;
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

#[cfg(test)]
mod tests {
    use std::path::Path;

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
    async fn a_root_holds_the_runner_the_command_packages_the_server_and_the_web_app_once() {
        let root = tempfile::tempdir().unwrap();
        let artifacts = root.path().join("artifacts");
        let linux = "x86_64-unknown-linux-musl";
        // Chrome for Testing has no Windows arm64 build, so demi-browser's
        // release of that target carries no archive to download.
        let windows = "aarch64-pc-windows-msvc";
        let mut commands = vec![Executable::Runner];
        commands.extend(Executable::COMMANDS);
        build(&artifacts, &commands, &[windows]);
        let web = root.path().join("dist");
        std::fs::create_dir_all(web.join("assets")).unwrap();
        std::fs::write(web.join(WEB_BUILD), r#"{"build":"fixture"}"#).unwrap();
        std::fs::write(web.join("assets/index.js"), "page").unwrap();
        let output = root.path().join("release");
        let options = Options {
            output: output.clone(),
            targets: vec![windows],
            server: Some(linux),
            web: Some(web),
            publish: false,
            artifacts: Some(artifacts.clone()),
            caches: Caches {
                compressed: Some(artifacts.join("compressed")),
            },
        };
        let cancel = CancellationToken::new();
        // Without the server's build, nothing is assembled and nothing is
        // left behind.
        let missing = assemble(&options, &cancel).await;
        assert!(
            matches!(missing, Err(Error::Native(native::Error::NotBuilt { .. }))),
            "{missing:?}"
        );
        assert_eq!(names(root.path()), ["artifacts", "dist"]);
        build(&artifacts, &[Executable::Backend, Executable::Machines], &[linux]);
        assemble(&options, &cancel).await.unwrap();
        assert_eq!(names(&output), ["bin", "commands", "runners", "web"]);
        assert_eq!(names(&output.join("bin")), ["demi-backend", "demi-machine-manager"]);
        assert_eq!(
            std::fs::read(output.join("bin/demi-backend")).unwrap(),
            format!("demi-backend {linux}").as_bytes()
        );
        assert_eq!(
            names(&output.join("commands")),
            ["demi-browser", "demi-claude-code", "demi-file"]
        );
        assert!(output.join("runners/manifest.json").is_file());
        assert_eq!(names(&output.join("web")), ["assets", WEB_BUILD]);
        // A root is assembled once.
        let again = assemble(&options, &cancel).await;
        assert!(matches!(again, Err(Error::Exists(_))), "{again:?}");
    }
}

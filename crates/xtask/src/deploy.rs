//! `xtask deploy` (`upgrades.md` § A development build on a server): moves
//! a server that runs Demi to a build of the checkout, without a release. It
//! asks the server's architecture, builds the server's Linux target and the
//! devices' targets here as the next patch's development pre-release,
//! assembles the server release as the release workflow does, copies it to
//! the server with rsync, has the server build its Cloud image with the
//! workflow's script, and runs `demi-server upgrade --from` there.
//! `--dry-run` builds and assembles here and prints what it would run on the
//! server instead.

use std::io::IsTerminal as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::time::{Duration, Instant};

use semver::{BuildMetadata, Prerelease, Version};

use crate::native::{self, BuildOptions, Caches, Executable};
use crate::server_release;

/// Where a server keeps its releases (`upgrades.md` § One release on a
/// server).
const RELEASES: &str = "/opt/demi/releases";
/// The directory of a development release's files in its root, which its
/// `release.json` names: no published location offers them.
const FILES: &str = "files";
/// Where the server holds a deploy's directory while it builds the image
/// and upgrades.
const STAGING: &str = "/var/tmp";
/// The image build's directory, relative to the repository and to the
/// deploy's directory; its `build.sh` finds its pins and overlay beside it.
const IMAGE_BUILD: &str = "cloud-guest-image/rootfs";
/// The built web app, relative to the repository.
const WEB: &str = "packages/web/dist";

#[derive(clap::Args)]
pub struct Options {
    /// The SSH host of the server, which runs Demi already and whose SSH user
    /// may use sudo [default: DEMI_DEPLOY_HOST in the checkout's .env].
    #[arg(long, env = "DEMI_DEPLOY_HOST", value_name = "HOST")]
    host: Option<String>,
    /// A target of the devices' runner and command programs, beside the
    /// server's own; repeat for several [default: this machine's].
    #[arg(long = "target", value_name = "TRIPLE", value_parser = native::target)]
    targets: Vec<&'static str>,
    /// The Apple SDK, which the Apple targets need.
    #[arg(long, env = "SDKROOT", value_name = "DIRECTORY")]
    sdk: Option<PathBuf>,
    /// Builds and assembles here, asks the server only its architecture,
    /// and prints what it would run there instead of running it.
    #[arg(long)]
    dry_run: bool,
}

/// Why a deploy stopped.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no server to deploy to: pass --host or set DEMI_DEPLOY_HOST in the checkout's .env")]
    NoHost,
    #[error("{host} is a {machine} machine: a Demi server runs x86_64 or aarch64 Linux")]
    Architecture { host: String, machine: String },
    #[error("{what} failed: {status}")]
    Step { what: String, status: ExitStatus },
    #[error(transparent)]
    Native(#[from] native::Error),
    #[error(transparent)]
    Assembly(#[from] server_release::Error),
    #[error("copying {}: {source}", path.display())]
    Copy {
        path: PathBuf,
        source: fs_extra::error::Error,
    },
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// The server's Linux target and the architecture its assets are named by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Server {
    target: &'static str,
    architecture: &'static str,
}

impl Server {
    /// The server whose `uname -m` printed `machine`.
    fn of(machine: &str) -> Option<Self> {
        match machine.trim() {
            "x86_64" | "amd64" => Some(Self {
                target: "x86_64-unknown-linux-musl",
                architecture: "amd64",
            }),
            "aarch64" | "arm64" => Some(Self {
                target: "aarch64-unknown-linux-musl",
                architecture: "arm64",
            }),
            _ => None,
        }
    }

    fn archive(self, version: &Version) -> String {
        format!("demi-{version}-server-linux-{}.tar.zst", self.architecture)
    }

    fn image_archive(self, version: &Version) -> String {
        format!("demi-{version}-image-linux-{}.tar", self.architecture)
    }
}

/// The version of a development build of the workspace version `workspace`
/// made at `now`: the next patch's pre-release named by the time, in UTC to
/// the minute. It sorts after every release before it and before the next
/// one, and after every earlier deploy.
fn development_version(workspace: &Version, now: jiff::Timestamp) -> Version {
    let time = now.strftime("%Y%m%dT%H%MZ").to_string();
    Version {
        major: workspace.major,
        minor: workspace.minor,
        patch: workspace.patch + 1,
        pre: Prerelease::new(&format!("dev.{time}")).expect("dev and a time are identifiers"),
        build: BuildMetadata::EMPTY,
    }
}

pub fn run(options: Options) -> Result<(), Error> {
    let host = options
        .host
        .filter(|host| !host.trim().is_empty())
        .ok_or(Error::NoHost)?;
    let repository = crate::repository();
    let workspace = Version::parse(env!("CARGO_PKG_VERSION")).expect("the workspace version is a version");
    let version = development_version(&workspace, jiff::Timestamp::now());
    let remote = Remote {
        host: &host,
        dry_run: options.dry_run,
    };
    let mut stages = Stages::default();

    let server = stages.time("Ask the server's architecture", || remote.architecture())?;
    let named = if options.targets.is_empty() {
        vec![native::target(demi_command_protocol::host_target()).map_err(std::io::Error::other)?]
    } else {
        options.targets
    };
    let mut devices: Vec<&'static str> = Vec::new();
    for target in named {
        if target != server.target && !devices.contains(&target) {
            devices.push(target);
        }
    }
    let mut targets = vec![server.target];
    targets.extend(&devices);
    println!("Deploying {version} to {host} ({})", server.target);

    stages.time("Build the server's programs", || {
        let mut executables = vec![Executable::Backend, Executable::Machines, Executable::Server];
        executables.extend(Executable::DEFAULT);
        build(executables, vec![server.target], None, &version)
    })?;
    if !devices.is_empty() {
        stages.time("Build the devices' programs", || {
            build(Executable::DEFAULT.to_vec(), devices.clone(), options.sdk.clone(), &version)
        })?;
    }
    let xtask = stages.time("Build xtask for the server", || {
        Ok(native::xtask(server.target, Some(&version.to_string()))?)
    })?;
    stages.time("Build the web app", || {
        local("bun install", Command::new("bun").args(["install", "--frozen-lockfile"]).current_dir(&repository))?;
        local("bun run build", Command::new("bun").args(["run", "build"]).current_dir(&repository))
    })?;

    let work = tempfile::Builder::new().prefix("demi-deploy-").tempdir()?;
    let stage = work.path().join("stage");
    let assembly = Assembly {
        work: work.path(),
        stage: &stage,
        version: &version,
        server,
        targets,
        web: repository.join(WEB),
        artifacts: None,
        caches: Caches { compressed: None },
        xtask: &xtask,
        image_build: repository.join(IMAGE_BUILD),
    };
    let archive = stages.time("Assemble the release", || assembly.assemble())?;
    let size = std::fs::metadata(&archive)?.len();

    let directory = format!("{STAGING}/demi-deploy-{version}");
    if let Err(error) = remote.deploy(&mut stages, &stage, &directory, server, &version) {
        if !options.dry_run {
            eprintln!("The release stays in {host}:{directory}; `ssh {host} sudo rm -rf {directory}` removes it");
        }
        return Err(error);
    }

    println!();
    if options.dry_run {
        println!("Built {version} (dry run: nothing changed on {host})");
    } else {
        println!("Deployed {version} to {host}");
    }
    println!(
        "  The server's {}: demi-backend, demi-machine-manager, demi-server, the runner and the command programs",
        server.target
    );
    if !devices.is_empty() {
        println!("  The devices' {}: the runner and the command programs", devices.join(", "));
    }
    println!("  The web app, and xtask for the server's image build");
    println!(
        "  {} ({:.1} MiB)",
        archive.file_name().expect("an archive has a name").to_string_lossy(),
        size as f64 / f64::from(1 << 20)
    );
    if options.dry_run {
        // What a dry run assembled stays for a look at it.
        let kept = work.keep();
        println!("  Assembled in {}", kept.display());
    }
    println!();
    stages.report();
    println!();
    println!("demi-server status on {host}:");
    remote.status()
}

/// Builds `executables` for `targets` with the deploy's version, as `xtask
/// native build` does.
fn build(
    executables: Vec<Executable>,
    targets: Vec<&'static str>,
    sdk: Option<PathBuf>,
    version: &Version,
) -> Result<(), Error> {
    native::run(native::Command::Build(BuildOptions {
        packages: executables,
        targets,
        artifacts: None,
        sdk,
        container: None,
        version: Some(version.to_string()),
    }))?;
    Ok(())
}

/// What a deploy assembles in `stage`, the directory it copies to the
/// server: the release's root with its files in `files/`, the server archive
/// in `dist/`, and `xtask` with the image build's directory, which the
/// server builds the image with.
struct Assembly<'a> {
    /// A directory for what the assembly makes on the way.
    work: &'a Path,
    stage: &'a Path,
    version: &'a Version,
    server: Server,
    targets: Vec<&'static str>,
    web: PathBuf,
    artifacts: Option<PathBuf>,
    caches: Caches,
    xtask: &'a Path,
    image_build: PathBuf,
}

impl Assembly<'_> {
    /// Assembles the stage and returns the server archive's path.
    fn assemble(&self) -> Result<PathBuf, Error> {
        let root = self.stage.join("root");
        let files = self.work.join(FILES);
        server_release::run(server_release::Options {
            output: root.clone(),
            files: files.clone(),
            // The files lie in the root, where the server unpacks them.
            downloads: Some(format!("{RELEASES}/{}/{FILES}", self.version)),
            targets: self.targets.clone(),
            server: Some(self.server.target),
            web: Some(self.web.clone()),
            publish: false,
            artifacts: self.artifacts.clone(),
            caches: self.caches.clone(),
            version: Some(self.version.to_string()),
        })?;
        std::fs::rename(&files, root.join(FILES))?;
        let dist = self.stage.join("dist");
        std::fs::create_dir(&dist)?;
        let archive = dist.join(self.server.archive(self.version));
        pack(&root, &archive)?;
        std::fs::copy(self.xtask, self.stage.join("xtask"))?;
        let image_build = self.stage.join(IMAGE_BUILD);
        std::fs::create_dir_all(&image_build)?;
        let contents = fs_extra::dir::CopyOptions::new().content_only(true);
        fs_extra::dir::copy(&self.image_build, &image_build, &contents).map_err(|source| Error::Copy {
            path: self.image_build.clone(),
            source,
        })?;
        Ok(archive)
    }
}

/// Writes the tree at `root` as the zstd-compressed tar archive `archive`,
/// as `tar --zstd -cf <archive> -C <root> .` does in the release workflow.
fn pack(root: &Path, archive: &Path) -> std::io::Result<()> {
    let file = std::fs::File::create(archive)?;
    // Level 0 is zstd's default, the one `tar --zstd` takes.
    let mut builder = tar::Builder::new(zstd::Encoder::new(file, 0)?);
    builder.follow_symlinks(false);
    builder.append_dir_all(".", root)?;
    builder.into_inner()?.finish()?.sync_all()
}

/// The server over SSH, or, in a dry run, what would run there.
struct Remote<'a> {
    host: &'a str,
    dry_run: bool,
}

impl Remote<'_> {
    /// The server's Linux target, which a dry run asks too: `uname -m`
    /// changes nothing.
    fn architecture(&self) -> Result<Server, Error> {
        let output = Command::new("ssh").arg(self.host).args(["uname", "-m"]).output()?;
        if !output.status.success() {
            return Err(Error::Step {
                what: format!("ssh {} uname -m", self.host),
                status: output.status,
            });
        }
        let machine = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        Server::of(&machine).ok_or_else(|| Error::Architecture {
            host: self.host.to_owned(),
            machine,
        })
    }

    /// Copies `stage` into `directory` on the server, builds the image
    /// there, moves the server to `version` and removes `directory`.
    fn deploy(
        &self,
        stages: &mut Stages,
        stage: &Path,
        directory: &str,
        server: Server,
        version: &Version,
    ) -> Result<(), Error> {
        let mut source = stage.as_os_str().to_owned();
        source.push("/");
        let mut copy = Command::new("rsync");
        copy.args(["--archive", "--compress"])
            .arg(source)
            .arg(format!("{}:{directory}/", self.host));
        self.stage(stages, "Copy the release to the server", || self.run("rsync", copy))?;
        let image = self.ssh(&[
            format!(
                "sudo bash {directory}/{IMAGE_BUILD}/build.sh --xtask {directory}/xtask --release {directory}/root --files {directory}/root/{FILES} --output {directory}/out/image"
            ),
            format!("tar -cf {directory}/dist/{} -C {directory}/out image", server.image_archive(version)),
            format!("cd {directory}/dist"),
            "sha256sum demi-* > SHA256SUMS".to_owned(),
        ]);
        self.stage(stages, "Build the Cloud image on the server", || self.run("the image build", image))?;
        let upgrade = self.ssh(&[format!("sudo demi-server upgrade {version} --from {directory}/dist")]);
        self.stage(stages, "Upgrade the server", || self.run("demi-server upgrade", upgrade))?;
        let cleanup = self.ssh(&[format!("sudo rm -rf {directory}")]);
        self.stage(stages, "Remove the release's copy from the server", || self.run("the cleanup", cleanup))
    }

    /// Prints `demi-server status` of the server.
    fn status(&self) -> Result<(), Error> {
        self.run("demi-server status", self.ssh(&["sudo demi-server status".to_owned()]))
    }

    /// Takes the stage `stage` on the server: timed, or in a dry run, where
    /// it only prints what it would run, untimed.
    fn stage<T>(
        &self,
        stages: &mut Stages,
        stage: &'static str,
        work: impl FnOnce() -> Result<T, Error>,
    ) -> Result<T, Error> {
        if self.dry_run {
            println!("== {stage}");
            return work();
        }
        stages.time(stage, work)
    }

    /// `ssh` running `steps` one after another on the server, each only if
    /// the one before succeeded, with a terminal when this one has one,
    /// where `sudo` asks for a password.
    fn ssh(&self, steps: &[String]) -> Command {
        let mut command = Command::new("ssh");
        if std::io::stdin().is_terminal() {
            command.arg("-t");
        }
        command.arg(self.host).arg(steps.join(" && "));
        command
    }

    /// Runs `command`, or prints it in a dry run.
    fn run(&self, what: &str, mut command: Command) -> Result<(), Error> {
        if self.dry_run {
            println!("Would run: {}", shown(&command));
            return Ok(());
        }
        local(what, &mut command)
    }
}

/// Runs `command` here and fails naming `what` when it fails.
fn local(what: &str, command: &mut Command) -> Result<(), Error> {
    let status = command.status()?;
    if !status.success() {
        return Err(Error::Step {
            what: what.to_owned(),
            status,
        });
    }
    Ok(())
}

/// `command` as a shell line, each argument with a space or a shell
/// character quoted.
fn shown(command: &Command) -> String {
    let mut line = command.get_program().to_string_lossy().into_owned();
    for argument in command.get_args() {
        let argument = argument.to_string_lossy();
        line.push(' ');
        if argument
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-_./:=@+,".contains(character))
        {
            line.push_str(&argument);
        } else {
            line.push('\'');
            line.push_str(&argument.replace('\'', r"'\''"));
            line.push('\'');
        }
    }
    line
}


/// How long each stage of the deploy took, in order.
#[derive(Default)]
struct Stages(Vec<(&'static str, Duration)>);

impl Stages {
    fn time<T>(&mut self, stage: &'static str, work: impl FnOnce() -> Result<T, Error>) -> Result<T, Error> {
        println!("== {stage}");
        let started = Instant::now();
        let result = work();
        self.0.push((stage, started.elapsed()));
        result
    }

    fn report(&self) {
        let width = self.0.iter().map(|(stage, _)| stage.len()).max().unwrap_or(0);
        let mut total = Duration::ZERO;
        for (stage, took) in &self.0 {
            println!("  {stage:width$}  {}", duration(*took));
            total += *took;
        }
        println!("  {:width$}  {}", "Total", duration(total));
    }
}

/// `took` as minutes and seconds, or seconds to a tenth under a minute.
fn duration(took: Duration) -> String {
    let seconds = took.as_secs_f64();
    if seconds < 60.0 {
        format!("{seconds:.1} s")
    } else {
        format!("{} min {:02} s", took.as_secs() / 60, took.as_secs() % 60)
    }
}

#[cfg(test)]
mod tests {
    use demi_runner_protocol::release::{SERVER_RELEASE, ServerRelease};

    use super::*;

    #[test]
    fn a_development_version_is_the_next_patch_named_by_the_minute() {
        let workspace = Version::new(0, 1, 21);
        let at = |time: &str| development_version(&workspace, time.parse().unwrap());
        let morning = at("2026-10-08T09:30:59Z");
        assert_eq!(morning.to_string(), "0.1.22-dev.20261008T0930Z");
        // After the release it follows and every earlier deploy, before the
        // release it leads to.
        assert!(morning > workspace);
        assert!(at("2026-10-08T10:05:00Z") > morning);
        assert!(at("2026-10-11T00:00:00Z") > at("2026-10-08T23:59:00Z"));
        assert!(morning < Version::new(0, 1, 22));
    }

    #[test]
    fn a_server_is_known_by_its_machine() {
        assert_eq!(Server::of("x86_64\n").unwrap().target, "x86_64-unknown-linux-musl");
        assert_eq!(Server::of("aarch64").unwrap().architecture, "arm64");
        assert_eq!(Server::of("riscv64"), None);
    }

    /// The assembly gives the server what `demi-server upgrade --from`
    /// and the image build read: the server archive by its asset name,
    /// whose root holds its programs and its files where its record names
    /// them, and the image build's script with an xtask beside the root.
    #[test]
    fn a_deploy_assembles_the_server_archive_and_the_image_builds_inputs() {
        let directory = tempfile::tempdir().unwrap();
        let artifacts = directory.path().join("artifacts");
        let server = Server::of("x86_64").unwrap();
        let device = "aarch64-apple-darwin";
        for (executable, targets) in [
            (Executable::Backend, &[server.target][..]),
            (Executable::Machines, &[server.target]),
            (Executable::Server, &[server.target]),
            (Executable::Runner, &[server.target, device]),
            (Executable::File, &[server.target, device]),
            (Executable::Browser, &[server.target, device]),
            (Executable::Claude, &[server.target, device]),
        ] {
            for target in targets {
                let built = native::built(&artifacts, executable, target);
                std::fs::create_dir_all(built.parent().unwrap()).unwrap();
                std::fs::write(&built, format!("{} {target}", executable.name())).unwrap();
            }
        }
        let web = directory.path().join("web");
        std::fs::create_dir_all(&web).unwrap();
        std::fs::write(web.join("build.json"), r#"{"build":"fixture"}"#).unwrap();
        let xtask = directory.path().join("xtask");
        std::fs::write(&xtask, "xtask").unwrap();
        let version: Version = "0.1.22-dev.20261008T0930Z".parse().unwrap();
        let work = directory.path().join("work");
        std::fs::create_dir(&work).unwrap();
        let stage = work.join("stage");
        let assembly = Assembly {
            work: &work,
            stage: &stage,
            version: &version,
            server,
            targets: vec![server.target, device],
            web,
            artifacts: Some(artifacts.clone()),
            caches: Caches {
                compressed: Some(artifacts.join("compressed")),
            },
            xtask: &xtask,
            image_build: crate::repository().join(IMAGE_BUILD),
        };
        let archive = assembly.assemble().unwrap();
        assert_eq!(archive, stage.join("dist/demi-0.1.22-dev.20261008T0930Z-server-linux-amd64.tar.zst"));
        assert!(stage.join(IMAGE_BUILD).join("build.sh").exists());
        assert_eq!(std::fs::read(stage.join("xtask")).unwrap(), b"xtask");
        assert!(!work.join(FILES).exists());

        // Unpacked as `demi-server` unpacks it.
        let unpacked = directory.path().join("unpacked");
        std::fs::create_dir(&unpacked).unwrap();
        let mut entries = tar::Archive::new(zstd::Decoder::new(std::fs::File::open(&archive).unwrap()).unwrap());
        entries.set_preserve_permissions(true);
        entries.unpack(&unpacked).unwrap();
        for program in ["demi-backend", "demi-machine-manager", "demi-server"] {
            assert_eq!(
                std::fs::read_to_string(unpacked.join("bin").join(program)).unwrap(),
                format!("{program} {}", server.target)
            );
        }
        let record = ServerRelease::decode(&std::fs::read(unpacked.join(SERVER_RELEASE)).unwrap()).unwrap();
        assert_eq!(record.files, "/opt/demi/releases/0.1.22-dev.20261008T0930Z/files");
        let mut files: Vec<String> = std::fs::read_dir(unpacked.join(FILES))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        files.sort();
        assert_eq!(files.len(), 8, "{files:?}");
        assert!(files.contains(&"demi-runner-aarch64-apple-darwin.zst".to_owned()), "{files:?}");
        // The command packages carry the deploy's version as the workspace
        // version.
        let descriptor: serde_json::Value = serde_json::from_slice(
            &std::fs::read(unpacked.join("commands/demi-file/descriptor.json")).unwrap(),
        )
        .unwrap();
        assert!(
            descriptor["version"]
                .as_str()
                .unwrap()
                .starts_with("0.1.22-dev.20261008T0930Z+dev."),
            "{descriptor}"
        );
        assert!(unpacked.join("web/build.json").exists());
    }
}

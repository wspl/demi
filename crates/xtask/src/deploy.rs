//! `xtask deploy` (`upgrades.md` § A development build on a server): moves
//! a server that runs Demi to a build of the checkout, without a release. It
//! asks the server, in one SSH call, its architecture and whether it has
//! what the image build needs; builds the server's Linux target and the
//! devices' targets here, and the web app, as the next patch's development
//! pre-release; assembles the server release as the release workflow does
//! and uploads its archive with rsync; then, in one SSH session, unpacks the
//! archive there, builds the Cloud image with the workflow's script, runs
//! `demi-server upgrade --from`, removes what it uploaded and prints
//! `demi-server status`. `--dry-run` builds and assembles here, asks the
//! server only the same questions, which change nothing, and prints what it
//! would run there.

use std::io::{BufRead as _, BufReader, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use semver::{BuildMetadata, Prerelease, Version};

use crate::native::{self, BuildOptions, Caches, Executable};
use crate::server_release;

/// Where a server keeps its releases (`upgrades.md` § One release on a
/// server).
const RELEASES: &str = "/opt/demi/releases";
/// The installed `demi-server` (`upgrades.md` § One release on a server),
/// by its path: sudo's search path may leave out `/usr/local/bin`.
const DEMI_SERVER: &str = "/usr/local/bin/demi-server";
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
/// What the server's programs, and the image build (`build.sh`: curl, jq,
/// GNU tar, util-linux, coreutils), need there, each with the apt package
/// that installs it. `sudo` is needed only by a user other than root.
const TOOLS: [(&str, &str); 10] = [
    ("curl", "curl"),
    ("jq", "jq"),
    ("zstd", "zstd"),
    ("tar", "tar"),
    ("rsync", "rsync"),
    ("sha256sum", "coreutils"),
    ("chroot", "coreutils"),
    ("mountpoint", "util-linux"),
    ("mount", "mount"),
    ("sudo", "sudo"),
];
/// What a line of the remote session starts with when the session begins a
/// stage, the stage's name after it.
const STAGE_MARK: &str = "::demi-deploy:: ";
/// The remote session's last stage, whose output is the server's status.
const STATUS: &str = "status";

#[derive(clap::Args)]
pub struct Options {
    /// The SSH host of the server, which runs Demi already, as root or as a
    /// user who may use sudo without a password [default: DEMI_DEPLOY_HOST in
    /// the checkout's .env].
    #[arg(long, env = "DEMI_DEPLOY_HOST", value_name = "HOST")]
    host: Option<String>,
    /// A target of the devices' runner and command programs, beside the
    /// server's own; repeat for several [default: this machine's].
    #[arg(long = "target", value_name = "TRIPLE", value_parser = native::target)]
    targets: Vec<&'static str>,
    /// The Apple SDK, which the Apple targets need [default: the one xcrun
    /// --show-sdk-path names on a Mac].
    #[arg(long, env = "SDKROOT", value_name = "DIRECTORY")]
    sdk: Option<PathBuf>,
    /// Builds and assembles here, asks the server only its architecture and
    /// tools, and prints what it would run there instead of running it.
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
    #[error("{host} has no {DEMI_SERVER}: a deploy moves a server that runs Demi already (installation.md)")]
    NotAServer { host: String },
    #[error(
        "{host} lacks {}, which the deploy needs there; install with `{install}` and deploy again",
        tools.join(", ")
    )]
    MissingTools {
        host: String,
        tools: Vec<String>,
        install: String,
    },
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
/// the second. It sorts after every release before it and before the next
/// one, and after every earlier deploy.
fn development_version(workspace: &Version, now: jiff::Timestamp) -> Version {
    let time = now.strftime("%Y%m%dT%H%M%SZ").to_string();
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
    let workspace =
        Version::parse(demi_shared_artifacts::WORKSPACE_VERSION).expect("the workspace version is a version");
    let version = development_version(&workspace, jiff::Timestamp::now());
    let remote = Remote {
        host: &host,
        dry_run: options.dry_run,
    };
    let mut stages = Stages::default();

    let mut probe = stages.time("Ask the server", || remote.probe())?;
    let server = probe.server;
    // A deploy stops before it builds; a dry run goes on, to show the rest.
    if !options.dry_run
        && let Some(lacking) = probe.lacking.take()
    {
        return Err(lacking);
    }
    if let Some(lacking) = &probe.lacking {
        println!("A deploy would stop here: {lacking}");
    }
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
    stages.time("Build the web app", || web(&repository))?;

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
    let status = match remote.deploy(&mut stages, &stage, &directory, server, &version) {
        Ok(status) => status,
        Err(error) => {
            if !options.dry_run {
                eprintln!("What the deploy uploaded stays in {host}:{directory}; `ssh {host} sudo rm -rf {directory}` removes it");
            }
            return Err(error);
        }
    };

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
    println!("  The web app");
    println!("  xtask for the server's image build");
    println!(
        "  {} ({:.1} MiB)",
        archive.file_name().expect("an archive has a name").to_string_lossy(),
        size as f64 / f64::from(1 << 20)
    );
    if options.dry_run {
        // What a dry run assembled stays for a look at it.
        let kept = work.keep();
        println!("  Assembled in {}", kept.display());
        if let Some(lacking) = &probe.lacking {
            println!("  A deploy would stop before it builds: {lacking}");
        }
    }
    println!();
    stages.report();
    if let Some(status) = status {
        println!();
        println!("demi-server status on {host}:");
        print!("{status}");
    }
    Ok(())
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

/// Builds the web app as the release workflow does.
fn web(repository: &Path) -> Result<(), Error> {
    local(
        "bun install",
        Command::new("bun")
            .args(["install", "--frozen-lockfile"])
            .current_dir(repository),
    )?;
    local(
        "bun run build",
        Command::new("bun").args(["run", "build"]).current_dir(repository),
    )
}

/// What a deploy assembles. The release's root, with its files in
/// `files/`, lies in `work`; `stage`, which the deploy uploads, holds the
/// server archive in `dist/`, and `xtask` with the image build's
/// directory, which the server builds the image with.
struct Assembly<'a> {
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
        let root = self.work.join("root");
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
        std::fs::create_dir_all(&dist)?;
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
    /// Asks the server, in one call that changes nothing, its architecture
    /// and what it lacks of what the deploy needs there; a dry run asks too.
    fn probe(&self) -> Result<Probe, Error> {
        let tools: Vec<&str> = TOOLS.iter().map(|(tool, _)| *tool).collect();
        let script = format!(
            "uname -m
if [ \"$(id -u)\" -eq 0 ]; then echo root; fi
for tool in {}; do command -v \"$tool\" >/dev/null 2>&1 || echo \"missing $tool\"; done
tar --version 2>/dev/null | grep -q 'GNU tar' || echo 'missing tar'
[ -x {DEMI_SERVER} ] || echo 'missing demi-server'
",
            tools.join(" ")
        );
        let output = self.session(&script)?.wait_with_output()?;
        if !output.status.success() {
            return Err(Error::Step {
                what: format!("asking {} its architecture and tools", self.host),
                status: output.status,
            });
        }
        probed(self.host, &String::from_utf8_lossy(&output.stdout))
    }

    /// Uploads `stage` into `directory` on the server, then, in one
    /// session, unpacks the release there, builds the image, moves the
    /// server to `version`, removes `directory` and reads the server's
    /// status, which it returns; none in a dry run.
    fn deploy(
        &self,
        stages: &mut Stages,
        stage: &Path,
        directory: &str,
        server: Server,
        version: &Version,
    ) -> Result<Option<String>, Error> {
        let mut source = stage.as_os_str().to_owned();
        source.push("/");
        let mut upload = Command::new("rsync");
        upload
            .args(["--archive", "--compress"])
            .arg(source)
            .arg(format!("{}:{directory}/", self.host));
        let archive = server.archive(version);
        let image = server.image_archive(version);
        let script = format!(
            "set -eu
# root runs each step itself; another user through sudo, which fails rather
# than asks for a password.
as_root() {{ if [ \"$(id -u)\" -eq 0 ]; then \"$@\"; else sudo -n \"$@\"; fi; }}
cd {directory}
echo '{STAGE_MARK}Unpack the release on the server'
mkdir root
tar --zstd -xf dist/{archive} -C root
echo '{STAGE_MARK}Build the Cloud image on the server'
as_root bash {directory}/{IMAGE_BUILD}/build.sh --xtask {directory}/xtask --release {directory}/root --files {directory}/root/{FILES} --output {directory}/out/image
tar -cf dist/{image} -C out image
(cd dist && sha256sum demi-* > SHA256SUMS)
echo '{STAGE_MARK}Upgrade the server'
as_root {DEMI_SERVER} upgrade {version} --from {directory}/dist
echo '{STAGE_MARK}Remove the upload from the server'
cd /
as_root rm -rf {directory}
echo '{STAGE_MARK}{STATUS}'
as_root {DEMI_SERVER} status
"
        );
        if self.dry_run {
            println!("== Upload the release to the server");
            println!("Would run: {}", shown(&upload));
            println!("== Unpack, build the image, upgrade, clean up and read the status on the server");
            println!("Would run: ssh {} bash -s <<'SCRIPT'\n{script}SCRIPT", self.host);
            return Ok(None);
        }
        stages.time("Upload the release to the server", || local("rsync", &mut upload))?;
        self.staged(stages, &script).map(Some)
    }

    /// Runs `script` in one session on the server, printing its output and
    /// timing each stage it marks; returns the output of the status stage.
    fn staged(&self, stages: &mut Stages, script: &str) -> Result<String, Error> {
        let mut session = self.session(script)?;
        let output = session.stdout.take().expect("the session's output is piped");
        let mut current: Option<(String, Instant)> = None;
        let mut status = String::new();
        for line in BufReader::new(output).lines() {
            let line = line?;
            if let Some(stage) = line.strip_prefix(STAGE_MARK) {
                if let Some((name, started)) = current.take() {
                    stages.record(name, started.elapsed());
                }
                if stage != STATUS {
                    println!("== {stage}");
                }
                current = Some((stage.to_owned(), Instant::now()));
            } else if current.as_ref().is_some_and(|(name, _)| name == STATUS) {
                status.push_str(&line);
                status.push('\n');
            } else {
                println!("{line}");
            }
        }
        let exit = session.wait()?;
        let ended = current.take();
        if !exit.success() {
            let what = match ended {
                Some((stage, _)) => format!("{stage} on {}", self.host),
                None => format!("the session on {}", self.host),
            };
            return Err(Error::Step { what, status: exit });
        }
        Ok(status)
    }

    /// An SSH session on the server running `script`, which it reads from
    /// its input, with its output piped here. No terminal: nothing there
    /// asks for input.
    fn session(&self, script: &str) -> Result<std::process::Child, Error> {
        let mut session = Command::new("ssh")
            .arg(self.host)
            .args(["bash", "-s"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()?;
        let mut input = session.stdin.take().expect("the session's input is piped");
        input.write_all(script.as_bytes())?;
        // Dropping the input ends it, so the script's shell ends with it.
        drop(input);
        Ok(session)
    }
}

/// What the probe found: the server, and what it lacks of what the deploy
/// needs there, which stops a deploy.
struct Probe {
    server: Server,
    lacking: Option<Error>,
}

/// What the probe's `answer` says of the server.
fn probed(host: &str, answer: &str) -> Result<Probe, Error> {
    let mut lines = answer.lines();
    let machine = lines.next().unwrap_or_default().trim().to_owned();
    let server = Server::of(&machine).ok_or_else(|| Error::Architecture {
        host: host.to_owned(),
        machine,
    })?;
    let mut root = false;
    let mut missing: Vec<&str> = Vec::new();
    for line in lines {
        match line.trim().strip_prefix("missing ") {
            // GNU tar is asked for twice: as a program and as GNU's.
            Some(tool) if !missing.contains(&tool) => missing.push(tool),
            Some(_) => {}
            None => root |= line.trim() == "root",
        }
    }
    if missing.contains(&"demi-server") {
        return Err(Error::NotAServer { host: host.to_owned() });
    }
    // root runs every step itself.
    if root {
        missing.retain(|tool| *tool != "sudo");
    }
    if missing.is_empty() {
        return Ok(Probe { server, lacking: None });
    }
    let mut packages: Vec<&str> = Vec::new();
    for tool in &missing {
        let package = TOOLS
            .iter()
            .find(|(known, _)| known == tool)
            .map_or(*tool, |(_, package)| *package);
        if !packages.contains(&package) {
            packages.push(package);
        }
    }
    let sudo = if root { "" } else { "sudo " };
    let lacking = Error::MissingTools {
        host: host.to_owned(),
        tools: missing.iter().map(|tool| (*tool).to_owned()).collect(),
        install: format!("ssh {host} {sudo}apt-get install -y {}", packages.join(" ")),
    };
    Ok(Probe {
        server,
        lacking: Some(lacking),
    })
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
struct Stages(Vec<(String, Duration)>);

impl Stages {
    fn time<T>(&mut self, stage: &str, work: impl FnOnce() -> Result<T, Error>) -> Result<T, Error> {
        println!("== {stage}");
        let started = Instant::now();
        let result = work();
        self.record(stage.to_owned(), started.elapsed());
        result
    }

    fn record(&mut self, stage: String, took: Duration) {
        self.0.push((stage, took));
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
    fn a_development_version_is_the_next_patch_named_by_the_second() {
        let workspace = Version::new(0, 1, 21);
        let at = |time: &str| development_version(&workspace, time.parse().unwrap());
        let morning = at("2026-10-08T09:30:59Z");
        assert_eq!(morning.to_string(), "0.1.22-dev.20261008T093059Z");
        // After the release it follows and every earlier deploy, before the
        // release it leads to.
        assert!(morning > workspace);
        assert!(at("2026-10-08T09:31:00Z") > morning);
        assert!(at("2026-10-11T00:00:00Z") > at("2026-10-08T23:59:59Z"));
        assert!(morning < Version::new(0, 1, 22));
    }

    /// The probe's answer gives the server's target, and a server that
    /// lacks a tool stops the deploy with the tools and the apt command
    /// that installs them; sudo matters only to a user other than root.
    #[test]
    fn the_server_is_known_by_its_machine_and_stops_the_deploy_without_its_tools() {
        let probe = probed("hel1", "x86_64\nroot\nmissing sudo\n").unwrap();
        assert_eq!(probe.server.target, "x86_64-unknown-linux-musl");
        assert!(probe.lacking.is_none());
        assert_eq!(probed("arm", "aarch64\n").unwrap().server.architecture, "arm64");
        assert!(matches!(probed("risc", "riscv64\n"), Err(Error::Architecture { .. })));
        let lacking = |answer: &str| probed("hel1", answer).unwrap().lacking.unwrap().to_string();
        assert_eq!(
            lacking("x86_64\nroot\nmissing jq\nmissing chroot\nmissing sha256sum\nmissing tar\nmissing tar\n"),
            "hel1 lacks jq, chroot, sha256sum, tar, which the deploy needs there; install with `ssh hel1 apt-get install -y jq coreutils tar` and deploy again"
        );
        let user = lacking("x86_64\nmissing sudo\n");
        assert!(user.contains("`ssh hel1 sudo apt-get install -y sudo`"), "{user}");
        assert!(matches!(
            probed("box", "x86_64\nroot\nmissing demi-server\n"),
            Err(Error::NotAServer { .. })
        ));
    }

    /// The assembly gives the server what `demi-server upgrade --from`
    /// and the image build read: the server archive by its asset name,
    /// whose root holds its programs and its files where its record names
    /// them, and the image build's script with an xtask beside it.
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
        let version: Version = "0.1.22-dev.20261008T093059Z".parse().unwrap();
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
        assert_eq!(archive, stage.join("dist/demi-0.1.22-dev.20261008T093059Z-server-linux-amd64.tar.zst"));
        // The upload is the archive and what builds the image, not the root.
        let mut uploaded: Vec<String> = std::fs::read_dir(&stage)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        uploaded.sort();
        assert_eq!(uploaded, ["cloud-guest-image", "dist", "xtask"]);
        assert!(stage.join(IMAGE_BUILD).join("build.sh").exists());
        assert_eq!(std::fs::read(stage.join("xtask")).unwrap(), b"xtask");

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
        assert_eq!(record.files, "/opt/demi/releases/0.1.22-dev.20261008T093059Z/files");
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
                .starts_with("0.1.22-dev.20261008T093059Z+dev."),
            "{descriptor}"
        );
        assert!(unpacked.join("web/build.json").exists());
    }
}

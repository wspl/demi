//! `demi-server`: the server's program for its own installation. Its commands:
//!
//! - `upgrade` fetches a release beside the current one and hands the move to
//!   that release's own `demi-server`. With `--requested` it moves to the
//!   version an administrator's Update Now asked for, as `demi-upgrade.service`
//!   runs it when the backend writes the request (`notices`).
//! - `rollback` hands the move to the release the last upgrade replaced.
//! - `status` says where the server is.
//! - `runtime` fetches the gVisor version this release pins.
//! - `setup` installs a first release.
//!
//! A move interrupted at any step is finished by the next command. The program
//! downloads through the artifact library, changes nothing outside the layout
//! (`layout`), never edits the configuration, touches no data directory but
//! `snapshots/` and a restore, and asks systemd to start and stop only Demi's
//! two services.
//!
//! A server runs one release at a time, and every part of Demi on it follows
//! that release. Nothing negotiates versions: the backend, the machine manager,
//! the web app, the runners and the command programs of a deployment are always
//! built from one build version, so none of the wires between them accepts
//! another.
//!
//! Only these meet a part of another release, each a contract that a release
//! never breaks:
//!
//! - The release's assets: the installed `demi-server` downloads the next release
//!   by the asset names and the `SHA256SUMS` file of the release workflow, so
//!   their names and that file's format never change.
//! - The management socket's lines: the next release's `demi-server` tells the
//!   running backend the runner release that comes next and how far the
//!   upgrade is, so the lines' fields and steps are only ever added.
//! - A runner's update: a paired device's runner of any earlier release must
//!   understand that the backend serves another one and how to fetch it, so the
//!   release check before the runner's socket opens, its headers and the body of
//!   its 409 answer never change and fields are only added.
//! - Stored data: the databases, the manager's records and the object store's
//!   keys outlive every release, and a release reads what every earlier one wrote
//!   through its migrations. Objects are immutable, and a release never rewrites
//!   one or changes what a key means.
//! - A pinned Cloud base: the manager boots a Cloud on the base it is pinned to,
//!   which an earlier release built, so what it needs of a base is the base's
//!   format.
//! - The configuration file belongs to the installation: each program refuses a
//!   setting it does not read, so a renamed or removed setting is reported, never
//!   silently ignored.
//!
//! A server therefore upgrades from any release in the schema history to any
//! later one directly, since each release migrates from every earlier format. A
//! migration that moves a stored thing to its new place, such as a Claude Code
//! sign-in kept in an account's record, or a conversation outside a project in a
//! directory other than its home, stays until the history starts again with the
//! first formal release.

mod data;
mod fetch;
mod gvisor;
mod journal;
mod layout;
mod moving;
mod notices;
mod progress;
mod runners;
mod services;
mod settings;
mod setup;

use std::{
    io::{self, BufRead as _, Write as _},
    os::unix::process::CommandExt as _,
    path::PathBuf,
    process::{Command, ExitCode},
};

use clap::{Parser, Subcommand};
use semver::Version;
use tokio_util::sync::CancellationToken;

use demi_runner_protocol::release::UpgradeNotice;

use crate::{journal::Journal, layout::Layout, notices::Notices, services::Systemd, settings::Settings};

/// Installs, upgrades and rolls back this Demi server.
#[derive(Parser)]
#[command(name = "demi-server", version = demi_shared_artifacts::BUILD_VERSION)]
struct Cli {
    #[command(subcommand)]
    command: Action,
    /// The directory every path lies beneath; `/` on a server.
    #[arg(long, global = true, hide = true, default_value = "/")]
    root: PathBuf,
}

#[derive(Subcommand)]
enum Action {
    /// Sets this Linux machine up as a Demi server.
    #[command(long_about = setup::GUIDE)]
    Setup(setup::Options),
    /// Moves the server to a newer release: the newest published one, or
    /// the one named.
    Upgrade {
        version: Option<Version>,
        /// Moves to the newest release, pre-releases included.
        #[arg(long, conflicts_with = "version")]
        prerelease: bool,
        /// Moves to the version the backend's request file names, as
        /// `demi-upgrade.service` asks.
        #[arg(long, hide = true, conflicts_with_all = ["version", "prerelease", "from"])]
        requested: bool,
        /// A directory holding the release's assets instead of the
        /// repository's GitHub releases.
        #[arg(long, value_name = "DIRECTORY")]
        from: Option<PathBuf>,
    },
    /// Returns the server to the release the last upgrade replaced.
    Rollback {
        /// Restores the databases without asking.
        #[arg(long)]
        yes: bool,
    },
    /// Prints the current release, the one a rollback returns to, and an
    /// interrupted move.
    Status,
    /// Fetches the gVisor runtime this release's machine manager pins, unless
    /// it is there, and prints its directory.
    Runtime,
    /// Carries out the move to this program's own release; `upgrade` and
    /// `rollback` start it.
    #[command(hide = true)]
    Move,
    /// Finishes an interrupted move to this program's own release.
    #[command(hide = true)]
    Resume,
}

fn main() -> ExitCode {
    // A panic leaves its report on standard error
    // (`crates/xtask/src/native/build.rs`).
    demi_shared_cli::install_panic_hook();
    let cli = Cli::parse();
    let layout = Layout::new(cli.root.clone());
    let requested = matches!(cli.command, Action::Upgrade { requested: true, .. });
    let result = match cli.command {
        Action::Status => status(&layout),
        Action::Setup(options) => setup(&layout, options),
        // A setup or a move runs it holding the lock; a fetch publishes
        // its version whole or not at all.
        Action::Runtime => require_root(&layout).and_then(|()| runtime(&layout)),
        command => require_root(&layout).and_then(|()| locked(&layout, |layout| act(layout, command))),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            // The administrator who asked hears why, and the request goes:
            // the path unit would start this run again for it.
            if requested {
                let _ = std::fs::remove_file(layout.upgrade_request());
                Notices::new(&layout).tell(UpgradeNotice::Failed {
                    to: String::new(),
                    reason: error.to_string(),
                });
            }
            eprintln!("demi-server: {error}");
            ExitCode::FAILURE
        }
    }
}

/// This program's release.
fn own_version() -> Version {
    Version::parse(demi_shared_artifacts::BUILD_VERSION).expect("the build version is a version")
}

/// Runs `work` holding the lock that keeps one `demi-server` at a time.
fn locked(
    layout: &Layout,
    work: impl FnOnce(&Layout) -> Result<(), Box<dyn std::error::Error>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let _lock = lock(layout)?;
    work(layout)
}

/// The lock that keeps one `demi-server` at a time, held until dropped.
fn lock(layout: &Layout) -> Result<std::fs::File, Box<dyn std::error::Error>> {
    std::fs::create_dir_all(layout.state())?;
    let lock = std::fs::File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(layout.state().join("lock"))?;
    if let Err(std::fs::TryLockError::WouldBlock) = lock.try_lock() {
        return Err("another demi-server is running".into());
    }
    Ok(lock)
}

/// Requires root on a server; a test's layout under another root runs as
/// anyone.
fn require_root(layout: &Layout) -> Result<(), Box<dyn std::error::Error>> {
    if layout.root() == std::path::Path::new("/") && !rustix::process::geteuid().is_root() {
        return Err("run as root".into());
    }
    Ok(())
}

fn setup(layout: &Layout, options: setup::Options) -> Result<(), Box<dyn std::error::Error>> {
    match setup::run(layout, &Systemd, options)? {
        setup::Outcome::SetUp => Ok(()),
        setup::Outcome::Guide => {
            let mut command = <Cli as clap::CommandFactory>::command();
            let setup = command
                .find_subcommand_mut("setup")
                .expect("setup is a subcommand");
            println!("{}", setup.render_long_help());
            Ok(())
        }
    }
}

fn runtime(layout: &Layout) -> Result<(), Box<dyn std::error::Error>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let directory = runtime.block_on(gvisor::fetch(layout, &CancellationToken::new()))?;
    println!("{}", directory.display());
    Ok(())
}

fn act(layout: &Layout, command: Action) -> Result<(), Box<dyn std::error::Error>> {
    // An interrupted move comes first, finished by its own release.
    if !matches!(command, Action::Resume)
        && let Some(journal) = Journal::read(&layout.journal())?
    {
        println!("Finishing the interrupted move from {} to {}", journal.from, journal.to);
        return Err(hand_over(layout, &journal.to, "resume").into());
    }
    match command {
        Action::Upgrade { version, prerelease, from, requested } => {
            let version = match (version, requested) {
                (_, true) => match requested_version(layout)? {
                    Some(version) => Some(version),
                    None => {
                        println!("No update was requested");
                        return Ok(());
                    }
                },
                (version, false) => version,
            };
            upgrade(layout, version, prerelease, from)
        }
        Action::Rollback { yes } => rollback(layout, yes),
        Action::Move => {
            let mut notices = Notices::new(layout);
            if let Err(failure) = moving::start(layout, &Systemd, &own_version(), &mut notices) {
                notices.tell(UpgradeNotice::Failed {
                    to: own_version().to_string(),
                    reason: failure.reason(),
                });
                return Err(failure.into());
            }
            println!("The server runs {}", own_version());
            Ok(())
        }
        Action::Resume => {
            let journal = Journal::read(&layout.journal())?.ok_or("no move to finish")?;
            let mut notices = Notices::new(layout);
            if let Err(failure) = moving::carry_out(layout, &Systemd, journal, &mut notices) {
                notices.tell(UpgradeNotice::Failed {
                    to: own_version().to_string(),
                    reason: failure.reason(),
                });
                return Err(failure.into());
            }
            println!("The server runs {}", own_version());
            Ok(())
        }
        Action::Status | Action::Setup(_) | Action::Runtime => unreachable!("runs without the lock"),
    }
}

impl std::error::Error for moving::Failure {}

fn upgrade(
    layout: &Layout,
    version: Option<Version>,
    prerelease: bool,
    from: Option<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    let source = match from {
        Some(directory) => fetch::Source::Directory(std::path::absolute(directory)?),
        None => fetch::Source::GitHub,
    };
    let current = layout.current_version()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let cancel = CancellationToken::new();
    let version = match version {
        Some(version) => version,
        None => runtime.block_on(fetch::newest(&source, prerelease))?,
    };
    if version == current {
        println!("The server runs {version} already");
        return Ok(());
    }
    let mut notices = Notices::new(layout);
    let fetched = if version < current {
        Err(format!("{version} is older than {current}: `demi-server rollback` goes back").into())
    } else {
        println!("Fetching {version}");
        notices.downloaded(&version, 0, 0);
        runtime
            .block_on(fetch::fetch(layout, &source, &version, &cancel, &mut |done, bytes| {
                notices.downloaded(&version, done, bytes);
            }))
            .map_err(Box::<dyn std::error::Error>::from)
    };
    if let Err(error) = fetched {
        notices.tell(UpgradeNotice::Failed {
            to: version.to_string(),
            reason: error.to_string(),
        });
        return Err(error);
    }
    // The next release tells the backend from here on.
    drop(notices);
    Err(hand_over(layout, &version, "move").into())
}

/// The version the backend's request names, which this run removes: none
/// when there is no request. The backend's user owns the request's
/// directory, so the file is opened without following a link and without
/// waiting, as opening a FIFO would wait for a writer forever, and it is a
/// plain file that holds a version and nothing else.
fn requested_version(layout: &Layout) -> Result<Option<Version>, Box<dyn std::error::Error>> {
    use std::{io::Read as _, os::unix::fs::OpenOptionsExt as _};
    /// A version is far shorter; a longer file is no request.
    const LONGEST: u64 = 256;
    let path = layout.upgrade_request();
    let file = std::fs::File::options()
        .read(true)
        .custom_flags((rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK).bits() as i32)
        .open(&path);
    let file = match file {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("{}: {error}", path.display()).into()),
    };
    std::fs::remove_file(&path)?;
    if !file.metadata()?.is_file() {
        return Err(format!("{} is not a file", path.display()).into());
    }
    let mut text = String::new();
    file.take(LONGEST).read_to_string(&mut text)?;
    let version = Version::parse(text.trim())
        .map_err(|error| format!("{} names no version: {error}", path.display()))?;
    Ok(Some(version))
}

fn rollback(layout: &Layout, yes: bool) -> Result<(), Box<dyn std::error::Error>> {
    let current = layout.current_version()?;
    let earlier = layout
        .versions()?
        .into_iter()
        .filter(|version| version < &current)
        .max()
        .ok_or("there is no earlier release to return to")?;
    let backend = Settings::read(&layout.config())?.backend_data()?;
    let snapshot = data::snapshot(&backend, &earlier);
    if snapshot.exists() && !yes {
        let taken = std::fs::metadata(&snapshot)?.modified()?;
        let taken = jiff::Timestamp::try_from(taken)?;
        print!(
            "Returning to {earlier} restores its databases as they were at {taken}: what Demi wrote since is set aside. Continue? [y/N] "
        );
        io::stdout().flush()?;
        let mut answer = String::new();
        io::stdin().lock().read_line(&mut answer)?;
        if !answer.trim().eq_ignore_ascii_case("y") {
            return Err("nothing changed".into());
        }
    }
    data::separate(&backend, &earlier)?;
    Err(hand_over(layout, &earlier, "move").into())
}

/// Runs `action` of `version`'s own `demi-server` in this process's place;
/// it returns only an error.
fn hand_over(layout: &Layout, version: &Version, action: &str) -> io::Error {
    let program = layout.release(version).join("bin/demi-server");
    Command::new(program)
        .arg(action)
        .arg("--root")
        .arg(layout.root())
        .exec()
}

fn status(layout: &Layout) -> Result<(), Box<dyn std::error::Error>> {
    let current = layout.current_version()?;
    println!("Current release: {current}");
    let earlier = layout
        .versions()?
        .into_iter()
        .filter(|version| version < &current)
        .max();
    match earlier {
        Some(earlier) => {
            let backend = Settings::read(&layout.config())?.backend_data()?;
            let restores = data::snapshot(&backend, &earlier).exists();
            let what = if restores {
                "and restores the databases it left"
            } else {
                "with the databases as they are"
            };
            println!("Rollback: returns to {earlier} {what}");
        }
        None => println!("Rollback: no earlier release"),
    }
    if let Some(journal) = Journal::read(&layout.journal())? {
        println!(
            "Interrupted: the move from {} to {} at {:?}",
            journal.from, journal.to, journal.step
        );
    }
    Ok(())
}

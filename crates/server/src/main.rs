//! `demi-server` (`upgrades.md`): the server's program for its own
//! installation. `upgrade` fetches a release beside the current one and hands
//! the move to that release's own `demi-server`; `rollback` hands it to the
//! release the last upgrade replaced; `status` says where the server is;
//! `runtime` fetches the gVisor version this release pins. A move
//! interrupted at any step is finished by the next command.

mod data;
mod fetch;
mod gvisor;
mod journal;
mod layout;
mod moving;
mod services;
mod settings;
mod setup;
#[cfg(test)]
mod tests;

use std::{
    io::{self, BufRead as _, Write as _},
    os::unix::process::CommandExt as _,
    path::PathBuf,
    process::{Command, ExitCode},
};

use clap::{Parser, Subcommand};
use semver::Version;
use tokio_util::sync::CancellationToken;

use crate::{journal::Journal, layout::Layout, services::Systemd, settings::Settings};

/// Installs, upgrades and rolls back this Demi server.
#[derive(Parser)]
#[command(name = "demi-server", version)]
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
    let cli = Cli::parse();
    let layout = Layout::new(cli.root.clone());
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
            eprintln!("demi-server: {error}");
            ExitCode::FAILURE
        }
    }
}

/// This program's release.
fn own_version() -> Version {
    Version::parse(env!("CARGO_PKG_VERSION")).expect("the workspace version is a version")
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
        Action::Upgrade { version, from } => upgrade(layout, version, from),
        Action::Rollback { yes } => rollback(layout, yes),
        Action::Move => {
            moving::start(layout, &Systemd, &own_version())?;
            println!("The server runs {}", own_version());
            Ok(())
        }
        Action::Resume => {
            let journal = Journal::read(&layout.journal())?.ok_or("no move to finish")?;
            moving::carry_out(layout, &Systemd, journal)?;
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
        None => runtime.block_on(fetch::newest(&source))?,
    };
    if version == current {
        println!("The server runs {version} already");
        return Ok(());
    }
    if version < current {
        return Err(format!("{version} is older than {current}: `demi-server rollback` goes back").into());
    }
    println!("Fetching {version}");
    runtime.block_on(fetch::fetch(layout, &source, &version, &cancel))?;
    Err(hand_over(layout, &version, "move").into())
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

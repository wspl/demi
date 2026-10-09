//! The repository's development commands (`crates-and-packages.md`
//! § xtask). `bun xtask` builds the workspace and runs them
//! (`builds-and-releases.md`).

#[cfg(test)]
mod boundaries;
mod browser;
mod cloud_image;
mod deploy;
// Generating the contracts reads the backend's plugins, whose library builds
// only where the backend runs; on Windows xtask builds the native releases.
#[cfg(all(unix, feature = "developer"))]
mod contracts;
#[cfg(all(unix, feature = "developer"))]
mod dev;
mod native;
mod server_release;
// Reads Unix file metadata: the sweep runs where development builds do.
#[cfg(unix)]
mod sweep;
mod vendor;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use tokio_util::sync::CancellationToken;

#[derive(Parser)]
#[command(name = "xtask", about = "The repository's development commands")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Generates the web app's TypeScript contracts from the Rust contract types.
    #[cfg(all(unix, feature = "developer"))]
    Contracts,
    /// Builds the native executables and packages their releases.
    #[command(subcommand)]
    Native(native::Command),
    /// Assembles a server release root from the built executables.
    ServerRelease(server_release::Options),
    /// Moves the server DEMI_DEPLOY_HOST names to a build of the checkout,
    /// without a release.
    Deploy(deploy::Options),
    /// Pins a Chrome for Testing version: writes its release record.
    BrowserRelease(browser::Options),
    /// Completes a Cloud image release on its Linux builder.
    #[command(subcommand)]
    CloudImage(cloud_image::Command),
    /// Compares the vendored crates with their upstream releases.
    #[command(subcommand)]
    Vendor(vendor::Command),
    /// Removes the build products no current build of the checkout uses.
    #[cfg(unix)]
    Sweep(sweep::Options),
    /// Runs a development backend with a scripted Cloud and the models `.env` turns on.
    #[cfg(all(unix, feature = "developer"))]
    Dev(dev::Options),
}

/// The repository's root directory.
fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("xtask sits two directories below the repository's root")
        .to_owned()
}

/// A release record's file: indented JSON with a final newline.
fn record(value: &impl serde::Serialize) -> serde_json::Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    Ok(bytes)
}

/// Runs `work` on a runtime of this thread with a token that an interrupt
/// (or, on Unix, a termination or a hang-up) cancels, so a publication it
/// makes stops and leaves nothing behind, and the processes `xtask dev`
/// started are stopped.
fn interruptible<T, F>(work: impl FnOnce(CancellationToken) -> F) -> std::io::Result<T>
where
    F: Future<Output = T>,
{
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        // The handlers are in place before `work` starts anything.
        let stop = stop_requested()?;
        let cancel = CancellationToken::new();
        let interrupt = tokio::spawn({
            let cancel = cancel.clone();
            async move {
                stop.await;
                cancel.cancel();
            }
        });
        let result = work(cancel).await;
        interrupt.abort();
        Ok(result)
    })
}

/// A future that ends when the user interrupts the command, or when it is
/// terminated or its terminal hangs up; the handlers it installs replace the
/// signals' default of ending the process at once.
#[cfg(unix)]
fn stop_requested() -> std::io::Result<impl Future<Output = ()>> {
    use tokio::signal::unix::{SignalKind, signal};
    let mut interrupt = signal(SignalKind::interrupt())?;
    let mut terminate = signal(SignalKind::terminate())?;
    let mut hangup = signal(SignalKind::hangup())?;
    Ok(async move {
        tokio::select! {
            _ = interrupt.recv() => {}
            _ = terminate.recv() => {}
            _ = hangup.recv() => {}
        }
    })
}

/// A future that ends when the user interrupts the command.
#[cfg(not(unix))]
fn stop_requested() -> std::io::Result<impl Future<Output = ()>> {
    Ok(async {
        // Without a handler, nothing interrupts the work; it runs to its end.
        if tokio::signal::ctrl_c().await.is_err() {
            std::future::pending::<()>().await;
        }
    })
}

/// Makes this process end as a termination ends it when the program that
/// started it ends: `bun xtask` runs it in a session of its own, which the
/// SIGKILL that stops a shell job's process group does not reach, and a
/// command that handles the termination, such as `xtask dev`, then stops
/// what it started.
#[cfg(target_os = "macos")]
fn end_with_parent() -> std::io::Result<()> {
    use std::mem::MaybeUninit;

    use rustix::event::kqueue::{Event, EventFilter, EventFlags, ProcessEvents, kevent, kqueue};
    use rustix::io::Errno;
    use rustix::process::{Signal, getpid, getppid, kill_process};

    // Without a parent there is nothing to follow.
    let Some(parent) = getppid() else {
        return Ok(());
    };
    let queue = kqueue()?;
    let exit = Event::new(
        EventFilter::Proc {
            pid: parent,
            flags: ProcessEvents::EXIT,
        },
        EventFlags::ADD | EventFlags::ONESHOT,
        std::ptr::null_mut(),
    );
    // With no room for events, the call only registers the change.
    let none: &mut [MaybeUninit<Event>] = &mut [];
    // SAFETY: the event refers to a process, not to a file descriptor.
    let registered = unsafe { kevent(&queue, &[exit], none, None) };
    match registered {
        Ok(_) => {}
        // The parent ended before the watch began.
        Err(Errno::SRCH) => return Ok(kill_process(getpid(), Signal::TERM)?),
        Err(error) => return Err(error.into()),
    }
    // The thread lives as long as the process, which releases it and its
    // queue when it ends.
    std::thread::spawn(move || {
        let mut events = [MaybeUninit::<Event>::uninit(); 1];
        loop {
            // SAFETY: as above, the queue holds only the process's event.
            match unsafe { kevent(&queue, &[], &mut events, None) } {
                Ok((ended, _)) if !ended.is_empty() => break,
                Ok(_) | Err(Errno::INTR) => {}
                Err(error) => {
                    eprintln!("xtask: cannot follow the program that started it: {error}");
                    return;
                }
            }
        }
        if let Err(error) = kill_process(getpid(), Signal::TERM) {
            eprintln!("xtask: cannot end with the program that started it: {error}");
        }
    });
    Ok(())
}

/// Makes this process end as a termination ends it when the program that
/// started it ends; see the macOS version. The kernel sends the signal when
/// the thread that started this process ends, so the parent has to start it
/// from a thread that lives as long as the parent does.
#[cfg(target_os = "linux")]
fn end_with_parent() -> std::io::Result<()> {
    use rustix::process::{Signal, getpid, getppid, kill_process, set_parent_process_death_signal};

    let parent = getppid();
    set_parent_process_death_signal(Some(Signal::TERM))?;
    // The parent ended before the signal was set.
    if getppid() != parent {
        kill_process(getpid(), Signal::TERM)?;
    }
    Ok(())
}

/// Elsewhere xtask does not follow the program that started it.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn end_with_parent() -> std::io::Result<()> {
    Ok(())
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    if let Err(error) = end_with_parent() {
        // xtask still runs; only the end of its parent no longer ends it.
        eprintln!("xtask: cannot follow the program that started it: {error}");
    }
    match cli.command {
        #[cfg(all(unix, feature = "developer"))]
        Command::Contracts => match contracts::run() {
            Ok(written) => {
                for path in written {
                    println!("wrote {}", path.display());
                }
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("xtask contracts: {error}");
                ExitCode::FAILURE
            }
        },
        Command::Native(command) => match native::run(command) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("xtask native: {error}");
                ExitCode::FAILURE
            }
        },
        Command::ServerRelease(options) => match server_release::run(options) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("xtask server-release: {error}");
                ExitCode::FAILURE
            }
        },
        Command::Deploy(options) => match deploy::run(options) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("xtask deploy: {error}");
                ExitCode::FAILURE
            }
        },
        Command::BrowserRelease(options) => match browser::run(options) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("xtask browser-release: {error}");
                ExitCode::FAILURE
            }
        },
        Command::CloudImage(command) => match cloud_image::run(command) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("xtask cloud-image: {error}");
                ExitCode::FAILURE
            }
        },
        Command::Vendor(command) => match vendor::run(command) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("xtask vendor: {error}");
                ExitCode::FAILURE
            }
        },
        #[cfg(unix)]
        Command::Sweep(options) => match sweep::run(options) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("xtask sweep: {error}");
                ExitCode::FAILURE
            }
        },
        #[cfg(all(unix, feature = "developer"))]
        Command::Dev(options) => match dev::run(options) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("xtask dev: {error}");
                ExitCode::FAILURE
            }
        },
    }
}

//! `run start` (`runner.md` § Installation, pairing and removal): starts the
//! installation's runner in the background again, as the installer does,
//! and returns once it is connected or has said why it cannot connect. A
//! runner already running is left as it is.

use std::{
    io,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use demi_runner_protocol::console::PAIRING_CODE;

use crate::{management, state::RunnerState};

/// How often the start looks at the runner it started.
const POLL: Duration = Duration::from_millis(100);

/// The lines the runner writes to its output when an attempt to connect
/// fails, each followed by why.
const FAILURES: [&str; 2] = [
    "demi-runner: connection ended: ",
    "demi-runner: registration refused ",
];

/// Starts the runner of the installation in `directory` with `arguments`,
/// the ones `run` takes, and says how it went: 0 once it is connected or was
/// running already, 1 when it stopped or cannot connect yet.
pub async fn start(directory: &Path, arguments: Vec<String>) -> io::Result<u8> {
    let state = RunnerState::open(directory.to_owned()).await?;
    match state.try_lock()? {
        // The active runner holds the installation's lock.
        None => {
            println!("The runner is running already.");
            return Ok(0);
        }
        Some(lease) => lease.release()?,
    }
    let log = directory.join("runner.log");
    let mut child = spawn(directory, &log, arguments)?;
    let mut read = 0;
    loop {
        if let Some(status) = child.try_wait()? {
            eprintln!("The runner stopped ({status}):");
            eprint!("{}", tokio::fs::read_to_string(&log).await.unwrap_or_default());
            return Ok(1);
        }
        let output = tokio::fs::read_to_string(&log).await?;
        // Only complete lines are read.
        let complete = output.rfind('\n').map_or(0, |end| end + 1);
        for line in output[read.min(complete)..complete].lines() {
            if let Some(code) = line.strip_prefix(PAIRING_CODE) {
                println!("The runner is not paired; enter this pairing code in Add Device: {code}");
                return Ok(1);
            }
            if let Some(reason) = FAILURES.iter().find_map(|prefix| line.strip_prefix(prefix)) {
                println!("The runner cannot connect: {reason}");
                println!("It keeps trying in the background.");
                return Ok(1);
            }
        }
        read = complete;
        if connected(&state).await {
            println!("The runner is connected.");
            return Ok(0);
        }
        tokio::time::sleep(POLL).await;
    }
}

/// Whether the installation's active runner says it is online; not before
/// it published its endpoint.
async fn connected(state: &RunnerState) -> bool {
    let mut answer = Vec::new();
    let asked = crate::manage(state, management::Action::Status, None, &mut answer).await;
    if !matches!(asked, Ok(0)) {
        return false;
    }
    serde_json::from_slice::<management::Status>(&answer)
        .is_ok_and(|status| status.phase == management::Phase::Online)
}

/// Starts the runner with `arguments` apart from this process, its output
/// in the installation's log as the installer makes it: private to the
/// user, since it holds pairing codes.
fn spawn(directory: &Path, log: &Path, arguments: Vec<String>) -> io::Result<tokio::process::Child> {
    let output = private_file(log)?;
    let mut command = tokio::process::Command::new(std::env::current_exe()?);
    command
        .args(arguments)
        .current_dir(home_or(directory))
        .stdin(Stdio::null());
    #[cfg(unix)]
    {
        // A group of its own: the terminal's interrupt and its closing do
        // not reach it.
        command.process_group(0);
        command.stdout(output.try_clone()?).stderr(output);
    }
    #[cfg(windows)]
    {
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
        // As the installer redirects it: the console lines in the log.
        let standard = private_file(&directory.join("runner.stdout.log"))?;
        command.stdout(standard).stderr(output);
    }
    command.spawn()
}

/// The user's home, where the installer starts the runner, or `directory`
/// when there is none.
fn home_or(directory: &Path) -> PathBuf {
    std::env::home_dir().unwrap_or_else(|| directory.to_owned())
}

/// `path`, emptied, readable and writable by its owner only.
fn private_file(path: &Path) -> io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options.open(path)
}

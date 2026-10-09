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
use tokio::io::{AsyncReadExt as _, AsyncSeekExt as _};

use crate::{console, management, state::RunnerState};

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
    let log = directory.join(console::LOG);
    // What the log holds already is an earlier runner's.
    let mut started = match tokio::fs::metadata(&log).await {
        Ok(metadata) => metadata.len(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => 0,
        Err(error) => return Err(error),
    };
    let mut child = spawn(directory, arguments)?;
    let mut read = started;
    loop {
        let (from, output) = since(&log, read).await?;
        if from < read {
            // The log started again past its size: what this runner wrote is
            // in the new one.
            started = from;
        }
        if let Some(status) = child.try_wait()? {
            let (_, output) = since(&log, started).await?;
            eprintln!("The runner stopped ({status}):");
            eprint!("{}", String::from_utf8_lossy(&output));
            return Ok(1);
        }
        // Only complete lines are read.
        let complete = output.iter().rposition(|byte| *byte == b'\n').map_or(0, |end| end + 1);
        for line in String::from_utf8_lossy(&output[..complete]).lines() {
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
        read = from + complete as u64;
        if connected(&state).await {
            println!("The runner is connected.");
            return Ok(0);
        }
        tokio::time::sleep(POLL).await;
    }
}

/// What the log at `log` holds from `offset` on, and the offset it was read
/// from: the log's start when the log is shorter, since it started again
/// past its size; nothing when there is no log yet.
async fn since(log: &Path, offset: u64) -> io::Result<(u64, Vec<u8>)> {
    let mut file = match tokio::fs::File::open(log).await {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok((offset, Vec::new())),
        Err(error) => return Err(error),
    };
    let from = if file.metadata().await?.len() < offset { 0 } else { offset };
    file.seek(io::SeekFrom::Start(from)).await?;
    let mut output = Vec::new();
    file.read_to_end(&mut output).await?;
    Ok((from, output))
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

/// Starts the runner with `arguments` apart from this process; it writes
/// its output to the installation's log itself.
fn spawn(directory: &Path, arguments: Vec<String>) -> io::Result<tokio::process::Child> {
    let mut command = tokio::process::Command::new(std::env::current_exe()?);
    command
        .args(arguments)
        .current_dir(home_or(directory))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        // A group of its own: the terminal's interrupt and its closing do
        // not reach it.
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
    }
    command.spawn()
}

/// The user's home, where the installer starts the runner, or `directory`
/// when there is none.
fn home_or(directory: &Path) -> PathBuf {
    std::env::home_dir().unwrap_or_else(|| directory.to_owned())
}

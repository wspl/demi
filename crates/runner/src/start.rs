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
/// the ones `run` takes, and says how it went: 0 once it is connected, 1
/// when it stopped or cannot connect yet. A runner running already is left
/// as it is, and asked to write its pairing state to its log again, which
/// the start reads as it reads a runner's it started.
pub async fn start(directory: &Path, arguments: Vec<String>) -> io::Result<u8> {
    let state = RunnerState::open(directory.to_owned()).await?;
    let lease = state.try_lock()?;
    let mut log = Follower::new(directory).await?;
    let mut child = match lease {
        Some(lease) => {
            lease.release()?;
            Some(spawn(directory, arguments)?)
        }
        // The active runner holds the installation's lock.
        None => {
            println!("The runner is running already.");
            // A runner that ends meanwhile says so below.
            let _ = crate::manage(&state, management::Action::Announce, None, tokio::io::sink()).await;
            None
        }
    };
    loop {
        let lines = log.lines().await?;
        let stopped = match &mut child {
            Some(child) => child.try_wait()?.map(|status| format!(" ({status})")),
            // A runner it found running released the lock as it ended.
            None => match state.try_lock()? {
                Some(lease) => {
                    lease.release()?;
                    Some(String::new())
                }
                None => None,
            },
        };
        if let Some(status) = stopped {
            eprintln!("The runner stopped{status}:");
            eprint!("{}", String::from_utf8_lossy(&log.written().await?));
            return Ok(1);
        }
        for line in lines.lines() {
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
        if connected(&state).await {
            println!("The runner is connected.");
            return Ok(0);
        }
        tokio::time::sleep(POLL).await;
    }
}

/// Reads the installation's log from where it ended when the start began
/// waiting: what it held before is an earlier runner's. It follows the log
/// when the runner starts it again past its size, finishing what was left
/// in the old one, `runner.log.1` by then, before reading the new one from
/// its start.
struct Follower {
    log: PathBuf,
    previous: PathBuf,
    /// Where the next read of the log starts: past the complete lines read.
    read: u64,
    /// The size `runner.log.1` had at the last read; another size means the
    /// log was moved there since.
    previous_size: u64,
    /// Where this runner's output begins in the log.
    begin: u64,
}

impl Follower {
    async fn new(directory: &Path) -> io::Result<Self> {
        let log = directory.join(console::LOG);
        let previous = directory.join(console::PREVIOUS_LOG);
        let read = size(&log).await?;
        Ok(Self {
            previous_size: size(&previous).await?,
            log,
            previous,
            read,
            begin: read,
        })
    }

    /// The complete lines written since the last read.
    async fn lines(&mut self) -> io::Result<String> {
        let mut text = Vec::new();
        let previous_size = size(&self.previous).await?;
        if size(&self.log).await? < self.read || previous_size != self.previous_size {
            take(&self.previous, self.read, &mut text).await?;
            self.read = 0;
            self.begin = 0;
        }
        self.previous_size = previous_size;
        self.read = take(&self.log, self.read, &mut text).await?;
        Ok(String::from_utf8_lossy(&text).into_owned())
    }

    /// What the runner wrote to the log since it began, or since the log
    /// started again.
    async fn written(&self) -> io::Result<Vec<u8>> {
        let mut file = match tokio::fs::File::open(&self.log).await {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        file.seek(io::SeekFrom::Start(self.begin)).await?;
        let mut written = Vec::new();
        file.read_to_end(&mut written).await?;
        Ok(written)
    }
}

/// The size of the file at `path`; 0 when there is none.
async fn size(path: &Path) -> io::Result<u64> {
    match tokio::fs::metadata(path).await {
        Ok(metadata) => Ok(metadata.len()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(error),
    }
}

/// Appends the complete lines of the file at `path` from `offset` on to
/// `text`, and returns the offset past them: `offset` itself when there is
/// no such file or no complete line past it.
async fn take(path: &Path, offset: u64, text: &mut Vec<u8>) -> io::Result<u64> {
    let mut file = match tokio::fs::File::open(path).await {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(offset),
        Err(error) => return Err(error),
    };
    if file.metadata().await?.len() <= offset {
        return Ok(offset);
    }
    file.seek(io::SeekFrom::Start(offset)).await?;
    let mut output = Vec::new();
    file.read_to_end(&mut output).await?;
    let complete = output.iter().rposition(|byte| *byte == b'\n').map_or(0, |end| end + 1);
    text.extend_from_slice(&output[..complete]);
    Ok(offset + complete as u64)
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

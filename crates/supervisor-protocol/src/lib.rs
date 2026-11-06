//! The socket of `demi-server supervise`, which runs a server's services in a
//! container, where no systemd runs, and the programs that ask it to act on
//! them: `demi-server` while it sets the server up or moves it to a release,
//! and the machine manager when it restarts itself.
//!
//! A connection carries one request and its answer, a line of JSON each way.
//! The socket meets programs of two releases: a move runs the next release's
//! `demi-server`, which asks the supervisor of the release before it until
//! that supervisor execs into the next release's. So a request's and an
//! answer's fields are only ever added, never renamed or removed.
//!
//! The supervisor execs only while no service runs, before a start: the
//! start's connection, and any other open then, closes unanswered, and
//! [`ask`] connects again and asks once more. A request that only a running
//! service's own process sends, a restart, never meets an exec, and every
//! other names the state it wants, so asking twice is asking once.

use std::{
    io::{self, BufRead as _, BufReader, Write as _},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};

/// Where the supervisor listens, for root alone.
pub const SOCKET: &str = "/run/demi-server/supervisor.sock";

/// What a program asks the supervisor.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Request {
    /// Stops `unit` and answers once its process ended and the unit's
    /// stop-post command ran.
    Stop { unit: String },
    /// Starts `unit` and answers once it reports ready, or fails.
    Start { unit: String },
    /// Reads the installed units again, and watches the files of the path
    /// units `paths` names and no others.
    Reload { paths: Vec<String> },
    /// The last lines `unit` wrote.
    Log { unit: String },
    /// Starts the backend `program` with the configuration `config` as the
    /// update screen of the steps in `record`, unless it runs.
    Screen {
        program: PathBuf,
        config: PathBuf,
        record: PathBuf,
    },
    /// Stops the update screen and answers once it ended.
    EndScreen,
    /// Stops `unit` and starts it again, answering at once: the unit's own
    /// process asks, and the stop signals it.
    Restart { unit: String },
}

/// The supervisor's answer.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Answer {
    Done,
    Log { text: String },
    Failed { error: String },
}

/// How long [`ask`] waits for a supervisor to listen again: an exec takes
/// well under a second, so a supervisor gone for this long has ended, and a
/// caller that waited on would wait forever.
const RETURN: Duration = Duration::from_secs(30);

/// Asks the supervisor listening on `socket`, asking again while it execs.
pub fn ask(socket: &Path, request: &Request) -> io::Result<Answer> {
    let deadline = Instant::now() + RETURN;
    loop {
        let error = match ask_once(socket, request) {
            Ok(Some(answer)) => return Ok(answer),
            Ok(None) => io::Error::other("the supervisor closed the connection unanswered"),
            Err(error) if matches!(error.kind(), io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused) => error,
            Err(error) => return Err(error),
        };
        if Instant::now() >= deadline {
            return Err(io::Error::other(format!("{}: {error}", socket.display())));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// The answer, none when the connection closed without one.
fn ask_once(socket: &Path, request: &Request) -> io::Result<Option<Answer>> {
    let mut stream = UnixStream::connect(socket)?;
    let mut line = serde_json::to_string(request).map_err(io::Error::other)?;
    line.push('\n');
    stream.write_all(line.as_bytes())?;
    let mut answer = String::new();
    if BufReader::new(stream).read_line(&mut answer)? == 0 {
        return Ok(None);
    }
    serde_json::from_str(&answer).map(Some).map_err(io::Error::other)
}

/// Reads a request from a connection the supervisor accepted, and writes
/// its answer there.
pub fn serve(stream: UnixStream, answer: impl FnOnce(Request) -> Answer) -> io::Result<()> {
    let mut line = String::new();
    BufReader::new(&stream).read_line(&mut line)?;
    let reply = match serde_json::from_str(&line) {
        Ok(request) => answer(request),
        Err(error) => Answer::Failed {
            error: format!("the supervisor reads no such request: {error}"),
        },
    };
    let mut line = serde_json::to_string(&reply).map_err(io::Error::other)?;
    line.push('\n');
    (&stream).write_all(line.as_bytes())
}

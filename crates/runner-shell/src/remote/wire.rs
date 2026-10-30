//! The job shell contract's frames on a shell's socket.
//!
//! A frame is a 4-byte big-endian length of what follows, a kind byte and a
//! body:
//!
//! - [`CONTROL`]: a JSON message, [`ToShell`] or [`FromShell`];
//! - [`OUTPUT`]: from the shell, a chunk of the job's output: the stream's
//!   byte, the chunk's 8-byte big-endian offset in its stream, and its bytes;
//! - [`INPUT`]: to the shell, bytes of the job's standard input.
//!
//! A frame of a kind a side does not know is skipped.

use std::{collections::BTreeMap, io, path::PathBuf};

use bytes::{BufMut, Bytes, BytesMut};
use demi_runner_protocol::wire::{OutputStream, Signal};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tokio::io::{AsyncRead, AsyncReadExt};

/// The contract's version, which changes only with a change an older side
/// would misread (`mod.rs`).
pub(super) const CONTRACT: u32 = 1;

const CONTROL: u8 = 0;
const OUTPUT: u8 = 1;
const INPUT: u8 = 2;

/// The most one frame holds, which a reader takes as a broken peer beyond:
/// a job's script and environment, the largest message, fit many times over.
const FRAME_BYTES: u32 = 64 * 1024 * 1024;

/// What the runner tells a job's shell.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum ToShell {
    /// The first message on each connection.
    Hello { contract: u32 },
    /// The job to run, on the first connection only.
    Start(Box<Start>),
    /// The job's standard input has ended.
    InputEnd,
    /// Read what the job's stdout pipe holds now; [`FromShell::Drained`]
    /// follows the output it held.
    Drain { id: u64 },
    Signal { signal: Signal },
    Cancel,
    /// Each stream's bytes the runner has kept, from the stream's start.
    Kept { stdout: u64, stderr: u64 },
    /// The backend has the job's exit, which the shell need hold no longer.
    ExitKept,
    /// The runner stops: the shell ends now, with what runs in it.
    End,
    /// Signal this shell's background task `task`, which another job's
    /// `kill` names; [`FromShell::TaskSignalled`] answers.
    SignalTask { request: u64, task: i32, signal: i32 },
    /// Whether a shell had the task this shell's
    /// [`FromShell::SignalTask`] named, and signalled it.
    TaskSignalled { request: u64, found: bool },
    #[serde(other)]
    Unknown,
}

/// A job the runner starts (`JobStart` in `crates/runner-process/src/job_shell.rs`).
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct Start {
    pub job_id: String,
    pub script: String,
    pub cwd: PathBuf,
    pub env: BTreeMap<String, String>,
    pub live: bool,
    #[serde(default)]
    pub commands: Option<Commands>,
    #[serde(default)]
    pub edits: Option<Edits>,
}

/// The job's declared commands, which go to the runner's local endpoint.
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct Commands {
    pub context: String,
    pub roots: Vec<String>,
    pub endpoint: String,
}

/// Where the job's edits are recorded (`demi_command_protocol::EditContext`).
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct Edits {
    pub directory: String,
    pub lock: String,
}

/// What a job's shell tells the runner.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum FromShell {
    /// The first message on each connection: what the shell holds is sent
    /// after it, from each stream's `kept` bytes on.
    Hello {
        contract: u32,
        release: String,
        /// Whether a runner started the shell's job.
        #[serde(default)]
        started: bool,
        /// Whether the shell's job has ended, and only what it left runs.
        #[serde(default)]
        ended: bool,
        stdout_kept: u64,
        stderr_kept: u64,
    },
    Started,
    StartFailed { reason: String },
    /// The job took one more chunk of its input.
    InputTaken,
    /// The job closed its standard input, and takes none more.
    InputClosed,
    Drained { id: u64 },
    Outliving { tasks: Vec<String> },
    /// The job's last output was sent before this.
    OutputEnd,
    Exit {
        code: Option<i32>,
        signal: Option<String>,
        error: Option<String>,
    },
    /// What the job left running has ended.
    Released,
    /// A line for the Host's log (`demi_command_sdk::log_lines_to`).
    Log { line: String },
    /// Signal background task `task`, which this shell's job's `kill` names
    /// and another job started: the runner asks every other shell.
    SignalTask { request: u64, task: i32, signal: i32 },
    /// Whether this shell had the task a [`ToShell::SignalTask`] named.
    TaskSignalled { request: u64, found: bool },
    #[serde(other)]
    Unknown,
}

/// A frame as a side reads it.
pub(super) enum Frame<C> {
    Control(C),
    Output {
        stream: OutputStream,
        offset: u64,
        bytes: Bytes,
    },
    Input(Bytes),
}

pub(super) fn control(message: &impl Serialize) -> io::Result<Bytes> {
    let body = serde_json::to_vec(message).map_err(io::Error::other)?;
    Ok(frame(CONTROL, &[], &body))
}

pub(super) fn output(stream: OutputStream, offset: u64, bytes: &[u8]) -> Bytes {
    let mut head = [0u8; 9];
    head[0] = stream_byte(stream);
    head[1..].copy_from_slice(&offset.to_be_bytes());
    frame(OUTPUT, &head, bytes)
}

pub(super) fn input(bytes: &[u8]) -> Bytes {
    frame(INPUT, &[], bytes)
}

fn frame(kind: u8, head: &[u8], body: &[u8]) -> Bytes {
    let length = 1 + head.len() + body.len();
    let mut frame = BytesMut::with_capacity(4 + length);
    frame.put_u32(u32::try_from(length).expect("a frame fits its length"));
    frame.put_u8(kind);
    frame.put_slice(head);
    frame.put_slice(body);
    frame.freeze()
}

/// The next frame, or none once the peer has closed the connection.
pub(super) async fn read<C: DeserializeOwned>(
    reader: &mut (impl AsyncRead + Unpin),
) -> io::Result<Option<Frame<C>>> {
    loop {
        let mut length = [0u8; 4];
        match reader.read_exact(&mut length).await {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(error) => return Err(error),
        }
        let length = u32::from_be_bytes(length);
        if length == 0 || length > FRAME_BYTES {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "a frame of no kind or beyond its bound"));
        }
        let mut body = vec![0u8; length as usize];
        reader.read_exact(&mut body).await?;
        let body = Bytes::from(body);
        match body[0] {
            CONTROL => {
                let message = serde_json::from_slice(&body[1..]).map_err(io::Error::other)?;
                return Ok(Some(Frame::Control(message)));
            }
            OUTPUT if body.len() >= 10 => {
                let stream = match body[1] {
                    0 => OutputStream::Stdout,
                    _ => OutputStream::Stderr,
                };
                let offset = u64::from_be_bytes(body[2..10].try_into().expect("eight bytes"));
                return Ok(Some(Frame::Output {
                    stream,
                    offset,
                    bytes: body.slice(10..),
                }));
            }
            INPUT => return Ok(Some(Frame::Input(body.slice(1..)))),
            // A frame of a kind this side does not know.
            _ => {}
        }
    }
}

fn stream_byte(stream: OutputStream) -> u8 {
    match stream {
        OutputStream::Stdout => 0,
        OutputStream::Stderr => 1,
    }
}

/// A stream's place in a pair of counts, stdout first.
pub(super) fn index(stream: OutputStream) -> usize {
    usize::from(stream_byte(stream))
}

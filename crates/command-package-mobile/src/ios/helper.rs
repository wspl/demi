//! The iOS helper's protocol (`crates/command-package-mobile/src/ios/mod.rs`):
//! `demi-ios-sim serve` speaks over its standard input and output in the live
//! view's framing, a four-byte big-endian length, a kind and the payload.
//!
//! - Requests and their replies are JSON, as are the events the helper sends
//!   unasked.
//! - A capture's pictures are video frames.
//! - A reply that carries a file, a PNG or an MP4, comes after its bytes in a
//!   blob frame.
//!
//! ```text
//! demi-mobile                         demi-ios-sim serve
//!   {id 4, method: tap, params} ───▶
//!                               ◀─── {id 4, result: {}}
//!                               ◀─── {event: drawn, udid}
//!                               ◀─── video: capture 2, sequence 17, …
//! ```

use std::{
    collections::HashMap,
    process::Stdio,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, AtomicU64, Ordering},
    },
};

use bytes::{Buf, BufMut, Bytes, BytesMut};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    process::Child,
    sync::{mpsc, oneshot},
};
use tokio_util::{sync::CancellationToken, task::AbortOnDropHandle};

use demi_live_view::stream::{CaptureEvent, EncodedFrame};

use crate::driver::DriverError;

/// A JSON message.
pub const CONTROL: u8 = 1;
/// A capture's picture: [`FrameHeader`], then H.264 Annex B.
pub const VIDEO: u8 = 2;
/// A reply's file: the request's id (u32), then its bytes.
pub const BLOB: u8 = 3;
/// The largest frame after its length.
const MAX_FRAME: usize = 64 * 1024 * 1024;

/// A video frame's header, big-endian: the capture (u32), the sequence
/// (u32), flags (u8, 1 = key frame), three reserved bytes, the timestamp in
/// microseconds (f64), the width and the height (u16 each).
pub const HEADER_BYTES: usize = 24;

/// What the helper says unasked.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "event", rename_all = "camelCase")]
pub enum Event {
    /// The device drew a new picture.
    Drawn { udid: String },
    /// The device went away.
    Lost { udid: String, reason: String },
    /// A capture failed or ended.
    Capture {
        capture: u32,
        #[serde(default)]
        failed: Option<String>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Reply {
    id: u64,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    error: Option<String>,
}

/// A reply, with the file that came before it.
pub struct Answer {
    pub result: Value,
    pub blob: Option<Bytes>,
}

type Pending = Mutex<HashMap<u64, oneshot::Sender<Result<Answer, DriverError>>>>;

/// How a helper is reached: its input, its output and, for a process,
/// the process, which the helper's end waits for.
pub struct Connection {
    pub input: Box<dyn AsyncWrite + Send + Unpin>,
    pub output: Box<dyn AsyncRead + Send + Unpin>,
    pub process: Option<Child>,
}

impl Connection {
    /// `executable serve`, its diagnostics going to the Host's log.
    pub fn spawn(executable: &std::path::Path) -> Result<Self, DriverError> {
        let mut child = tokio::process::Command::new(executable)
            .arg("serve")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| DriverError::Failed(format!("the iOS helper did not start: {error}")))?;
        let input = child.stdin.take().expect("piped stdin");
        let output = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        // A line at a time, until the helper's end closes it.
        tokio::spawn(async move {
            use tokio::io::AsyncBufReadExt;
            let mut lines = tokio::io::BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::info!("demi-ios-sim: {line}");
            }
        });
        Ok(Self {
            input: Box::new(input),
            output: Box::new(output),
            process: Some(child),
        })
    }
}

/// What the reading task and the callers share: the replies awaited, the
/// captures' frames, and the helper's end. It holds nothing of the
/// process, so a helper nobody holds ends.
#[derive(Default)]
struct Shared {
    pending: Pending,
    /// The captures' frames, by capture.
    captures: Mutex<HashMap<u32, mpsc::Sender<CaptureEvent>>>,
    /// Cancelled once the helper ended.
    ended: CancellationToken,
    /// Why it ended.
    reason: Mutex<Option<String>>,
}

impl Shared {
    fn lost(&self) -> DriverError {
        DriverError::Lost(
            self.reason
                .lock()
                .expect("the reason is intact")
                .clone()
                .unwrap_or_else(|| "the iOS helper stopped".into()),
        )
    }
}

/// How long a helper whose input closed may take to let its devices go,
/// as it does then, before it is killed.
const LETTING_GO: std::time::Duration = std::time::Duration::from_secs(10);

/// One running helper, which the conversation's devices share. Dropping
/// it closes its input, which the helper takes as its end: it lets its
/// devices go and exits, and its process is waited for.
pub struct Helper {
    input: tokio::sync::Mutex<Box<dyn AsyncWrite + Send + Unpin>>,
    shared: Arc<Shared>,
    next: AtomicU64,
    /// The helper's captures, of all its devices, take numbers from one
    /// count: the helper routes their frames by number alone.
    captures: AtomicU32,
    process: Mutex<Option<Child>>,
    _reading: AbortOnDropHandle<()>,
}

impl Helper {
    /// Serves over `connection`, whose events reach `events`.
    pub fn start(connection: Connection, events: mpsc::UnboundedSender<Event>) -> Arc<Self> {
        let shared = Arc::new(Shared::default());
        let reading = shared.clone();
        let output = connection.output;
        let task = tokio::spawn(async move {
            let ended = read(output, &reading, events).await;
            *reading.reason.lock().expect("the reason is intact") = Some(ended.clone());
            reading.ended.cancel();
            for (_, waiting) in reading.pending.lock().expect("the replies are intact").drain() {
                let _gone = waiting.send(Err(DriverError::Lost(ended.clone())));
            }
            reading.captures.lock().expect("the captures are intact").clear();
        });
        Arc::new(Self {
            input: tokio::sync::Mutex::new(connection.input),
            shared,
            next: AtomicU64::new(1),
            captures: AtomicU32::new(1),
            process: Mutex::new(connection.process),
            // The reading ends with the output, which ends with the helper;
            // it holds none of the helper's own, so the helper can end.
            _reading: AbortOnDropHandle::new(task),
        })
    }

    /// Cancelled once the helper ended.
    pub fn ended(&self) -> &CancellationToken {
        &self.shared.ended
    }

    /// A number for a new capture, unique among the helper's captures.
    pub fn next_capture(&self) -> u32 {
        self.captures.fetch_add(1, Ordering::Relaxed)
    }

    /// Asks `method` with `params` and waits for its answer.
    pub async fn ask(&self, method: &str, params: Value) -> Result<Answer, DriverError> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (answer, answered) = oneshot::channel();
        self.shared
            .pending
            .lock()
            .expect("the replies are intact")
            .insert(id, answer);
        let request = serde_json::to_vec(&json!({ "id": id, "method": method, "params": params }))
            .expect("a request serializes");
        let sent = {
            let mut input = self.input.lock().await;
            let mut frame = BytesMut::with_capacity(5 + request.len());
            frame.put_u32(u32::try_from(1 + request.len()).expect("a request fits a frame"));
            frame.put_u8(CONTROL);
            frame.put_slice(&request);
            match input.write_all(&frame).await {
                Ok(()) => input.flush().await,
                Err(error) => Err(error),
            }
        };
        if let Err(error) = sent {
            self.shared.pending.lock().expect("the replies are intact").remove(&id);
            return Err(DriverError::Lost(format!("the iOS helper stopped: {error}")));
        }
        tokio::select! {
            answer = answered => answer.unwrap_or_else(|_| Err(self.shared.lost())),
            _ = self.shared.ended.cancelled() => {
                self.shared.pending.lock().expect("the replies are intact").remove(&id);
                Err(self.shared.lost())
            }
        }
    }

    /// Asks and decodes the result as `T`.
    pub async fn call<T: DeserializeOwned>(&self, method: &str, params: Value) -> Result<T, DriverError> {
        let answer = self.ask(method, params).await?;
        serde_json::from_value(answer.result)
            .map_err(|error| DriverError::Failed(format!("the iOS helper answered {method} with what Demi cannot read: {error}")))
    }

    /// The frames of capture `capture` arrive at the receiver until the
    /// capture ends.
    pub fn frames(&self, capture: u32) -> mpsc::Receiver<CaptureEvent> {
        let (sender, frames) = mpsc::channel(8);
        self.shared
            .captures
            .lock()
            .expect("the captures are intact")
            .insert(capture, sender);
        frames
    }

    pub fn forget(&self, capture: u32) {
        self.shared.captures.lock().expect("the captures are intact").remove(&capture);
    }
}

impl Drop for Helper {
    fn drop(&mut self) {
        // The input closes with the helper's fields; the process gets the
        // time to let its devices go, then it is killed, and waited for
        // either way.
        let Some(mut process) = self.process.lock().expect("the process is intact").take() else {
            return;
        };
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            // Without a runtime the process is killed as its handle drops.
            return;
        };
        runtime.spawn(async move {
            match tokio::time::timeout(LETTING_GO, process.wait()).await {
                Ok(Ok(_)) => {}
                Ok(Err(error)) => tracing::warn!("the iOS helper could not be waited for: {error}"),
                Err(_) => {
                    tracing::warn!("the iOS helper did not end when its input closed; it is killed");
                    // Killing waits for it; a helper that is gone already is ended.
                    let _killed = process.kill().await;
                }
            }
        });
    }
}

/// Reads the helper's frames until it ends; answers why it ended.
async fn read(
    mut output: Box<dyn AsyncRead + Send + Unpin>,
    helper: &Shared,
    events: mpsc::UnboundedSender<Event>,
) -> String {
    let mut blobs: HashMap<u64, Bytes> = HashMap::new();
    let mut buffer = BytesMut::with_capacity(64 * 1024);
    loop {
        while buffer.len() >= 4 {
            let length = u32::from_be_bytes(buffer[..4].try_into().expect("four bytes")) as usize;
            if length == 0 || length > MAX_FRAME {
                return format!("the iOS helper sent a frame of {length} bytes");
            }
            if buffer.len() < 4 + length {
                break;
            }
            buffer.advance(4);
            let mut payload = buffer.split_to(length).freeze();
            let kind = payload.get_u8();
            match kind {
                CONTROL => {
                    if let Err(error) = control(&payload, helper, &events, &mut blobs) {
                        return error;
                    }
                }
                VIDEO => {
                    if payload.len() < HEADER_BYTES {
                        return "the iOS helper sent a video frame shorter than its header".into();
                    }
                    let mut header = payload.split_to(HEADER_BYTES);
                    let capture = header.get_u32();
                    let sequence = header.get_u32();
                    let key = header.get_u8() & 1 == 1;
                    header.advance(3);
                    let timestamp = header.get_f64();
                    let width = header.get_u16();
                    let height = header.get_u16();
                    let sender = helper
                        .captures
                        .lock()
                        .expect("the captures are intact")
                        .get(&capture)
                        .cloned();
                    if let Some(sender) = sender {
                        // A stream that has not taken the frame before is
                        // behind; its viewers resume from a key frame.
                        let _behind = sender.try_send(CaptureEvent::Frame(EncodedFrame {
                            sequence,
                            key,
                            timestamp,
                            width,
                            height,
                            data: if key { crate::pictures::without_reordering(payload) } else { payload },
                        }));
                    }
                }
                BLOB => {
                    if payload.len() < 4 {
                        return "the iOS helper sent a blob shorter than its id".into();
                    }
                    let id = u64::from(payload.get_u32());
                    blobs.insert(id, payload);
                }
                kind => return format!("the iOS helper sent a frame of kind {kind}"),
            }
        }
        match output.read_buf(&mut buffer).await {
            Ok(0) => return "the iOS helper exited".into(),
            Ok(_) => {}
            Err(error) => return format!("the iOS helper's output failed: {error}"),
        }
    }
}

fn control(
    payload: &[u8],
    helper: &Shared,
    events: &mpsc::UnboundedSender<Event>,
    blobs: &mut HashMap<u64, Bytes>,
) -> Result<(), String> {
    let value: Value = serde_json::from_slice(payload)
        .map_err(|error| format!("the iOS helper sent what is not JSON: {error}"))?;
    if value.get("event").is_some() {
        let event: Event = serde_json::from_value(value)
            .map_err(|error| format!("the iOS helper sent an event Demi cannot read: {error}"))?;
        if let Event::Capture { capture, failed } = &event {
            let sender = helper
                .captures
                .lock()
                .expect("the captures are intact")
                .remove(capture);
            if let (Some(sender), Some(failed)) = (sender, failed) {
                let _gone = sender.try_send(CaptureEvent::Failed(failed.clone()));
            }
        }
        // The driver is gone once the service stops.
        let _gone = events.send(event);
        return Ok(());
    }
    let reply: Reply = serde_json::from_value(value)
        .map_err(|error| format!("the iOS helper sent a reply Demi cannot read: {error}"))?;
    let waiting = helper
        .pending
        .lock()
        .expect("the replies are intact")
        .remove(&reply.id);
    let blob = blobs.remove(&reply.id);
    if let Some(waiting) = waiting {
        let answer = match (reply.result, reply.error) {
            (_, Some(error)) => Err(DriverError::Failed(error)),
            (result, None) => Ok(Answer {
                result: result.unwrap_or(Value::Null),
                blob,
            }),
        };
        // A caller that stopped waiting needs no answer.
        let _left = waiting.send(answer);
    }
    Ok(())
}

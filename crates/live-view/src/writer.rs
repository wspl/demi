//! What a viewer's page receives: control messages before video chunks, and a
//! heartbeat whenever nothing else was sent for 250 ms.
//!
//! The viewer's delivery hands chunks on at their paced times, within its
//! window, so video waits here only behind control messages and the path's own
//! backpressure.
//!
//! After one second without a byte the page shows the stream as stalled and
//! discards input rather than queueing it: a Cloud pausing for a checkpoint
//! must not deliver typed input all at once afterward. Silence for 75 seconds
//! is a broken stream; a shorter stall keeps the view.

use std::{marker::PhantomData, time::Duration};

use bytes::Bytes;
use serde::Serialize;
use tokio::{sync::mpsc, time::Instant};
use tokio_util::sync::CancellationToken;

use demi_command_sdk::Output;
use demi_live_view_protocol::{HEARTBEAT_MS, LiveHeartbeat, LiveNotice};
use demi_shared_types::DecodeError;

use crate::frames;

/// Control messages waiting for the page. The viewer waits for room here, so
/// a page that stops reading holds back the viewer, not memory.
const CONTROL_QUEUE: usize = 64;

/// A source's messages to the page as the writer sends them.
pub trait ModuleMessage:
    demi_live_view_protocol::ModuleMessage
    + Serialize
    + garde::Validate<Context = ()>
    + Send
    + Sync
    + 'static
{
}

impl<M> ModuleMessage for M where
    M: demi_live_view_protocol::ModuleMessage
        + Serialize
        + garde::Validate<Context = ()>
        + Send
        + Sync
        + 'static
{
}

pub struct Writer<M> {
    control: mpsc::Sender<Bytes>,
    /// Bounded by the delivery's window, never by this queue.
    video: mpsc::UnboundedSender<Bytes>,
    /// Ends a wait for room in the control queue; never, for the viewer's own.
    ended: CancellationToken,
    messages: PhantomData<fn(&M)>,
}

impl<M> Clone for Writer<M> {
    fn clone(&self) -> Self {
        Self {
            control: self.control.clone(),
            video: self.video.clone(),
            ended: self.ended.clone(),
            messages: PhantomData,
        }
    }
}

impl<M: ModuleMessage> Writer<M> {
    pub fn start(output: Output) -> (Self, tokio::task::JoinHandle<()>) {
        let (control, controls) = mpsc::channel(CONTROL_QUEUE);
        let (video, videos) = mpsc::unbounded_channel();
        let heartbeat = frames::control(&M::heartbeat(LiveHeartbeat {}));
        let task = tokio::spawn(run(output, controls, videos, heartbeat));
        (
            Self {
                control,
                video,
                ended: CancellationToken::new(),
                messages: PhantomData,
            },
            task,
        )
    }

    /// This writer for a task that may outlive the view, which its owner
    /// joins: a page that stops reading holds the task back only until
    /// `ended`, after which what it had to say no longer matters.
    pub fn until(&self, ended: &CancellationToken) -> Self {
        Self {
            ended: ended.clone(),
            ..self.clone()
        }
    }

    /// Queues a control message. A message outside the protocol's bounds
    /// would end the view on the page, so it is logged and not sent: it comes
    /// from a source's defect, such as a page observer's.
    pub async fn control(&self, message: &M) {
        if let Err(report) = garde::Validate::validate(message) {
            tracing::warn!("live view message refused: {}", DecodeError::from(report));
            return;
        }
        let bytes = frames::control(message);
        tokio::select! {
            biased;
            _ = self.ended.cancelled() => {}
            // The view ended, and its page with it.
            _ended = self.control.send(bytes) => {}
        }
    }

    /// Something the viewer asked for failed; the stream goes on.
    pub async fn notice(&self, code: impl std::fmt::Display, message: &str) {
        self.control(&M::notice(LiveNotice {
            code: code.to_string(),
            message: message.into(),
        }))
        .await;
    }

    /// Queues a video chunk; one for a view that ended goes nowhere.
    pub fn video(&self, chunk: Bytes) {
        // The view ended, and its page with it.
        let _ended = self.video.send(chunk);
    }
}

async fn run(
    output: Output,
    mut controls: mpsc::Receiver<Bytes>,
    mut videos: mpsc::UnboundedReceiver<Bytes>,
    heartbeat: Bytes,
) {
    let quiet = Duration::from_millis(HEARTBEAT_MS);
    let mut written = Instant::now();
    loop {
        let bytes = tokio::select! {
            biased;
            control = controls.recv() => match control {
                Some(bytes) => bytes,
                None => break,
            },
            video = videos.recv() => match video {
                Some(bytes) => bytes,
                None => break,
            },
            _ = tokio::time::sleep_until(written + quiet) => heartbeat.clone(),
        };
        if output.stdout(bytes).await.is_err() {
            break;
        }
        written = Instant::now();
    }
}

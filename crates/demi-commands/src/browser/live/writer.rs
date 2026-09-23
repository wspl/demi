//! What a viewer's page receives (`live-view.md` § Delivery): control
//! messages before video, video only while the path takes it, and a heartbeat
//! whenever nothing else was sent for a while.

use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use bytes::Bytes;
use demi_command_service::Output;
use tokio::{sync::mpsc, time::Instant};

use super::{
    super::protocol::{LIVE_HEARTBEAT_MS, LiveOutbound},
    frames,
};

/// Control messages waiting for the page. The viewer waits for room here, so
/// a page that stops reading holds back the viewer, not memory.
const CONTROL_QUEUE: usize = 64;
/// Video frames waiting for the page; past this the viewer drops frames and
/// resumes from a key frame.
const VIDEO_QUEUE: usize = 4;

#[derive(Clone)]
pub(super) struct Writer {
    control: mpsc::Sender<Bytes>,
    video: mpsc::Sender<Bytes>,
    queued: Arc<AtomicUsize>,
}

impl Writer {
    pub fn start(output: Output) -> (Self, tokio::task::JoinHandle<()>) {
        let (control, controls) = mpsc::channel(CONTROL_QUEUE);
        let (video, videos) = mpsc::channel(VIDEO_QUEUE);
        let queued = Arc::new(AtomicUsize::new(0));
        let task = tokio::spawn(run(output, controls, videos, queued.clone()));
        (
            Self {
                control,
                video,
                queued,
            },
            task,
        )
    }

    pub async fn control(&self, message: &LiveOutbound) {
        let _ended = self.control.send(frames::control(message)).await;
    }

    /// Something the viewer asked for failed; the stream goes on.
    pub async fn notice(&self, code: &str, message: &str) {
        self.control(&LiveOutbound::Notice {
            code: code.into(),
            message: message.into(),
        })
        .await;
    }

    /// Queues a video frame, or reports that the path has no room for it.
    pub fn video(&self, frame: Bytes) -> bool {
        let length = frame.len();
        self.queued.fetch_add(length, Ordering::SeqCst);
        if self.video.try_send(frame).is_err() {
            self.queued.fetch_sub(length, Ordering::SeqCst);
            return false;
        }
        true
    }

    /// Video bytes not yet written.
    pub fn queued(&self) -> usize {
        self.queued.load(Ordering::SeqCst)
    }
}

async fn run(
    output: Output,
    mut controls: mpsc::Receiver<Bytes>,
    mut videos: mpsc::Receiver<Bytes>,
    queued: Arc<AtomicUsize>,
) {
    let quiet = Duration::from_millis(LIVE_HEARTBEAT_MS);
    let heartbeat = frames::control(&LiveOutbound::Heartbeat {});
    let mut written = Instant::now();
    loop {
        let bytes = tokio::select! {
            biased;
            control = controls.recv() => match control {
                Some(bytes) => bytes,
                None => break,
            },
            video = videos.recv() => match video {
                Some(bytes) => {
                    queued.fetch_sub(bytes.len(), Ordering::SeqCst);
                    bytes
                }
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

//! One watched tab's pictures (`live-view.md` § Capture): viewers of
//! the same tab share one capture and one encoding. The capture follows the
//! tab's viewport, and its pace and encoding follow the viewer with the least
//! room.

use std::{collections::HashMap, sync::Arc, time::Duration};

use tokio::{sync::mpsc, time::Instant};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use super::{
    capture::{Capture, CaptureEvent, Captures, Frame},
    rate::{self, FRAME_RATES, SCALES},
};
use crate::browser::{BrowserError, BrowserTab};

/// Frames a viewer has not taken yet. The viewer drains them as they come
/// and drops what its page cannot take, so a full queue means the viewer
/// itself stopped.
const VIEWER_QUEUE: usize = 8;
const RETRY_LIMIT: Duration = Duration::from_secs(5);

pub(super) enum Event {
    /// Pictures of this size follow, starting with a key frame; `epoch`
    /// names this capture in the viewer's pace reports.
    Restart {
        epoch: u32,
        width: u32,
        height: u32,
    },
    Frame(Frame),
    /// This Host cannot capture, for this reason.
    Unavailable(String),
    /// The capture failed for this reason; the stream tries again.
    Failed(String),
}

pub(super) enum Command {
    Join {
        viewer: u64,
        events: mpsc::Sender<Event>,
    },
    Leave {
        viewer: u64,
    },
    /// The viewer has frames after `floor` in flight and allows `window`.
    Pace {
        viewer: u64,
        epoch: u32,
        floor: u32,
        window: u32,
    },
    Encoding {
        viewer: u64,
        bitrate: u32,
        fps: u32,
        scale: f64,
    },
    KeyFrame,
}

struct Viewer {
    events: mpsc::Sender<Event>,
    floor: u32,
    window: u32,
    /// None until the viewer measured its path.
    bitrate: Option<u32>,
    fps: u32,
    scale: f64,
}

/// Starts the tab's stream; it ends when the returned sender is dropped or
/// the tab closes.
pub(super) fn start(
    tab: BrowserTab,
    captures: Arc<Captures>,
    tasks: &TaskTracker,
) -> mpsc::UnboundedSender<Command> {
    let (commands, receiver) = mpsc::unbounded_channel();
    tasks.spawn(run(tab, captures, receiver));
    commands
}

pub(super) fn events() -> (mpsc::Sender<Event>, mpsc::Receiver<Event>) {
    mpsc::channel(VIEWER_QUEUE)
}

struct Running {
    capture: Capture,
    epoch: u32,
    size: (u32, u32),
    encoding: (u32, u32),
    /// The newest frame's sequence.
    sequence: u32,
}

async fn run(
    tab: BrowserTab,
    captures: Arc<Captures>,
    mut commands: mpsc::UnboundedReceiver<Command>,
) {
    let mut viewers: HashMap<u64, Viewer> = HashMap::new();
    let mut viewport = tab.state.viewport.subscribe();
    let mut running: Option<Running> = None;
    let mut epoch = 0;
    let mut retry = Duration::ZERO;
    let mut attempt = Instant::now();
    let mut unavailable: Option<String> = None;
    // Why the viewers get no picture, as they were last told.
    let mut failure: Option<String> = None;
    let stop = CancellationToken::new();
    let _stop = stop.clone().drop_guard();
    loop {
        let fps = viewers
            .values()
            .map(|viewer| viewer.fps)
            .min()
            .unwrap_or(FRAME_RATES[0]);
        let scale = viewers
            .values()
            .map(|viewer| viewer.scale)
            .fold(SCALES[0], f64::min);
        let size = crate::browser::viewport::pixels(&viewport.borrow_and_update().current, scale);
        // The slowest viewer's budget; before any measured one, the budget a
        // sharp picture of this size needs.
        let bitrate = viewers
            .values()
            .filter_map(|viewer| viewer.bitrate)
            .min()
            .unwrap_or_else(|| rate::initial_bitrate(u64::from(size.0) * u64::from(size.1), fps));
        let restart = running.as_ref().is_none_or(|running| running.size != size);
        if viewers.is_empty() {
            running = None;
        } else if restart && unavailable.is_none() && Instant::now() >= attempt {
            running = None;
            match captures
                .start(tab.target_id(), size.0, size.1, fps, bitrate, &stop)
                .await
            {
                Ok(capture) => {
                    epoch += 1;
                    for viewer in viewers.values_mut() {
                        viewer.floor = 0;
                        let _behind = viewer.events.try_send(Event::Restart {
                            epoch,
                            width: size.0,
                            height: size.1,
                        });
                    }
                    running = Some(Running {
                        capture,
                        epoch,
                        size,
                        encoding: (bitrate, fps),
                        sequence: 0,
                    });
                }
                // Waiting changes nothing on a Host that cannot capture.
                Err(BrowserError::UnsupportedCapability(reason)) => {
                    for viewer in viewers.values() {
                        let _behind = viewer.events.try_send(Event::Unavailable(reason.clone()));
                    }
                    unavailable = Some(reason);
                }
                Err(error) => {
                    eprintln!("live view capture of {}: {error}", tab.id());
                    report(&viewers, &mut failure, error.to_string());
                    retry = (retry * 2).clamp(Duration::from_millis(500), RETRY_LIMIT);
                    attempt = Instant::now() + retry;
                }
            }
        }
        if let Some(running) = &mut running
            && running.encoding != (bitrate, fps)
        {
            running.capture.encoding(bitrate, fps);
            running.encoding = (bitrate, fps);
        }
        let waiting = running.is_none() && unavailable.is_none() && !viewers.is_empty();
        let event = async {
            match running.as_mut() {
                Some(running) => running.capture.events.recv().await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            _ = tab.ended.cancelled() => return,
            _ = viewport.changed() => {}
            _ = tokio::time::sleep_until(attempt), if waiting => {}
            command = commands.recv() => {
                let Some(command) = command else { return };
                match command {
                    Command::Join { viewer, events } => {
                        if let Some(reason) = &unavailable {
                            let _behind = events.try_send(Event::Unavailable(reason.clone()));
                        }
                        if let Some(reason) = &failure {
                            let _behind = events.try_send(Event::Failed(reason.clone()));
                        }
                        if let Some(running) = &running {
                            let _behind = events.try_send(Event::Restart {
                                epoch: running.epoch,
                                width: running.size.0,
                                height: running.size.1,
                            });
                            running.capture.key_frame();
                        }
                        let floor = running.as_ref().map_or(0, |running| running.sequence);
                        viewers.insert(viewer, Viewer {
                            events,
                            floor,
                            window: 4,
                            bitrate: None,
                            fps: FRAME_RATES[0],
                            scale: SCALES[0],
                        });
                    }
                    Command::Leave { viewer } => {
                        viewers.remove(&viewer);
                    }
                    Command::Pace { viewer, epoch, floor, window } => {
                        let Some(running) = &running else { continue };
                        if epoch != running.epoch {
                            continue;
                        }
                        if let Some(entry) = viewers.get_mut(&viewer) {
                            (entry.floor, entry.window) = (floor, window);
                        }
                        let floor = viewers.values().map(|viewer| viewer.floor).min();
                        let window = viewers.values().map(|viewer| viewer.window).min();
                        if let (Some(floor), Some(window)) = (floor, window) {
                            running.capture.ack(floor, window);
                        }
                    }
                    Command::Encoding { viewer, bitrate, fps, scale } => {
                        if let Some(entry) = viewers.get_mut(&viewer) {
                            (entry.bitrate, entry.fps, entry.scale) = (Some(bitrate), fps, scale);
                        }
                    }
                    Command::KeyFrame => {
                        if let Some(running) = &running {
                            running.capture.key_frame();
                        }
                    }
                }
            }
            event = event => match event {
                Some(CaptureEvent::Frame(frame)) => {
                    // Enqueuing a start does not mean Chrome acquired its stream.
                    retry = Duration::ZERO;
                    if let Some(running) = &mut running {
                        running.sequence = frame.sequence;
                    }
                    failure = None;
                    for viewer in viewers.values() {
                        let _behind = viewer.events.try_send(Event::Frame(frame.clone()));
                    }
                }
                // A page that stopped painting before capture began sends no
                // picture; a screenshot from its surface paints one.
                Some(CaptureEvent::Stalled) => {
                    let page = tab.page.clone();
                    tokio::spawn(async move {
                        let _painted = tokio::time::timeout(
                            Duration::from_secs(5),
                            crate::browser::viewport::paint(&page),
                        )
                        .await;
                    });
                }
                Some(CaptureEvent::Started) => {}
                Some(CaptureEvent::Failed(message)) => {
                    eprintln!("live view capture of {}: {message}", tab.id());
                    report(&viewers, &mut failure, message);
                    running = None;
                    retry = (retry * 2).clamp(Duration::from_millis(500), RETRY_LIMIT);
                    attempt = Instant::now() + retry;
                }
                None => {
                    running = None;
                    retry = (retry * 2).clamp(Duration::from_millis(500), RETRY_LIMIT);
                    attempt = Instant::now() + retry;
                }
            },
        }
    }
}

/// Tells the viewers why they get no picture, once per reason: the capture
/// retries, and a repeated failure is not news.
fn report(viewers: &HashMap<u64, Viewer>, failure: &mut Option<String>, reason: String) {
    if failure.as_ref() == Some(&reason) {
        return;
    }
    for viewer in viewers.values() {
        let _behind = viewer.events.try_send(Event::Failed(reason.clone()));
    }
    *failure = Some(reason);
}

//! One watched tab's pictures (`live-view.md` § Capture): viewers of
//! the same tab share one capture and one encoding. The capture follows the
//! tab's viewport, and its pace and encoding follow the viewer with the least
//! room.
//!
//! Nothing reaches a stream through a queue. The hub publishes who watches
//! the tab; each viewer publishes its latest pacing, which supersedes the one
//! before, and wakes the stream to read it.

use std::{collections::HashMap, sync::Arc, time::Duration};

use tokio::{
    sync::{mpsc, watch},
    time::Instant,
};
use tokio_util::task::TaskTracker;

use super::{
    capture::{Capture, CaptureChannel, CaptureEvent, Frame},
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

/// What a viewer last told its tab's stream.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(super) struct Pacing {
    /// The capture epoch, the newest frame the page has or will never get,
    /// and how many frames may be in flight past it.
    pace: Option<(u32, u32, u32)>,
    /// The bit rate, frame rate and scale the viewer's path takes.
    encoding: Option<(u32, u32, f64)>,
    /// Counts the key frames the viewer asked for.
    key_frames: u64,
}

/// A viewer's side of the stream it watches.
pub(super) struct StreamView {
    pub events: mpsc::Receiver<Event>,
    pacing: watch::Sender<Pacing>,
    /// Wakes the stream to read its viewers' pacing.
    wake: watch::Sender<u64>,
}

impl StreamView {
    /// The viewer has frames after `floor` in flight and allows `window`.
    pub fn pace(&self, epoch: u32, floor: u32, window: u32) {
        self.change(|pacing| pacing.pace = Some((epoch, floor, window)));
    }

    pub fn encoding(&self, bitrate: u32, fps: u32, scale: f64) {
        self.change(|pacing| pacing.encoding = Some((bitrate, fps, scale)));
    }

    pub fn key_frame(&self) {
        self.change(|pacing| pacing.key_frames += 1);
    }

    fn change(&self, change: impl FnOnce(&mut Pacing)) {
        self.pacing.send_modify(change);
        self.wake
            .send_modify(|wakes| *wakes = wakes.wrapping_add(1));
    }
}

/// A viewer as its stream sees it.
#[derive(Clone)]
struct Member {
    events: mpsc::Sender<Event>,
    pacing: watch::Receiver<Pacing>,
}

type Members = Arc<HashMap<u64, Member>>;

/// The hub's side of one watched tab's stream; the stream ends when this is
/// dropped or the tab closes.
pub(super) struct StreamHandle {
    members: watch::Sender<Members>,
    wake: watch::Sender<u64>,
}

impl StreamHandle {
    pub fn start(tab: BrowserTab, captures: CaptureChannel, tasks: &TaskTracker) -> Self {
        let (members, watched) = watch::channel(Members::default());
        let (wake, woken) = watch::channel(0);
        tasks.spawn(run(tab, captures, watched, woken, tasks.clone()));
        Self { members, wake }
    }

    /// Adds viewer `id`; its view carries the pictures and its pacing.
    pub fn join(&self, id: u64) -> StreamView {
        let (events, receiver) = mpsc::channel(VIEWER_QUEUE);
        let (pacing, paced) = watch::channel(Pacing::default());
        self.members.send_modify(|members| {
            let mut next = HashMap::clone(members);
            next.insert(
                id,
                Member {
                    events,
                    pacing: paced,
                },
            );
            *members = Arc::new(next);
        });
        StreamView {
            events: receiver,
            pacing,
            wake: self.wake.clone(),
        }
    }

    pub fn leave(&self, id: u64) {
        self.members.send_if_modified(|members| {
            if !members.contains_key(&id) {
                return false;
            }
            let mut next = HashMap::clone(members);
            next.remove(&id);
            *members = Arc::new(next);
            true
        });
    }

    pub fn is_empty(&self) -> bool {
        self.members.borrow().is_empty()
    }
}

struct Viewer {
    events: mpsc::Sender<Event>,
    pacing: watch::Receiver<Pacing>,
    /// The key frame requests already served.
    key_frames: u64,
    floor: u32,
    window: u32,
    /// None until the viewer measured its path.
    bitrate: Option<u32>,
    fps: u32,
    scale: f64,
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
    captures: CaptureChannel,
    mut members: watch::Receiver<Members>,
    mut wake: watch::Receiver<u64>,
    tasks: TaskTracker,
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
    // Ends a capture start that waits for the extension when the tab closes
    // or the stream ends.
    let stop = tab.ended.child_token();
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
                    tracing::warn!("live view capture of {}: {error}", tab.id());
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
            changed = members.changed() => {
                // The hub drops a stream nobody watches.
                if changed.is_err() {
                    return;
                }
                let current = members.borrow_and_update().clone();
                viewers.retain(|id, _| current.contains_key(id));
                for (id, member) in current.iter() {
                    if viewers.contains_key(id) {
                        continue;
                    }
                    if let Some(reason) = &unavailable {
                        let _behind = member.events.try_send(Event::Unavailable(reason.clone()));
                    }
                    if let Some(reason) = &failure {
                        let _behind = member.events.try_send(Event::Failed(reason.clone()));
                    }
                    if let Some(running) = &running {
                        let _behind = member.events.try_send(Event::Restart {
                            epoch: running.epoch,
                            width: running.size.0,
                            height: running.size.1,
                        });
                        running.capture.key_frame();
                    }
                    viewers.insert(*id, Viewer {
                        events: member.events.clone(),
                        pacing: member.pacing.clone(),
                        key_frames: 0,
                        floor: running.as_ref().map_or(0, |running| running.sequence),
                        window: 4,
                        bitrate: None,
                        fps: FRAME_RATES[0],
                        scale: SCALES[0],
                    });
                }
            }
            woken = wake.changed() => {
                if woken.is_err() {
                    return;
                }
                paced(&mut viewers, running.as_ref());
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
                    let ended = tab.ended.clone();
                    tasks.spawn(async move {
                        tokio::select! {
                            _ = ended.cancelled() => {}
                            _painted = tokio::time::timeout(
                                Duration::from_secs(5),
                                crate::browser::viewport::paint(&page),
                            ) => {}
                        }
                    });
                }
                Some(CaptureEvent::Started) => {}
                Some(CaptureEvent::Failed(message)) => {
                    tracing::warn!("live view capture of {}: {message}", tab.id());
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

/// Takes in what the viewers told the stream since it last looked: the latest
/// pace and encoding of each, and whether one asked for a key frame.
fn paced(viewers: &mut HashMap<u64, Viewer>, running: Option<&Running>) {
    let mut key_frame = false;
    let mut acknowledged = false;
    for viewer in viewers.values_mut() {
        if !viewer.pacing.has_changed().unwrap_or(false) {
            continue;
        }
        let pacing = *viewer.pacing.borrow_and_update();
        if let Some((epoch, floor, window)) = pacing.pace
            && running.is_some_and(|running| running.epoch == epoch)
        {
            (viewer.floor, viewer.window) = (floor, window);
            acknowledged = true;
        }
        if let Some((bitrate, fps, scale)) = pacing.encoding {
            (viewer.bitrate, viewer.fps, viewer.scale) = (Some(bitrate), fps, scale);
        }
        if pacing.key_frames != viewer.key_frames {
            viewer.key_frames = pacing.key_frames;
            key_frame = true;
        }
    }
    let Some(running) = running else {
        return;
    };
    if acknowledged {
        let floor = viewers.values().map(|viewer| viewer.floor).min();
        let window = viewers.values().map(|viewer| viewer.window).min();
        if let (Some(floor), Some(window)) = (floor, window) {
            running.capture.ack(floor, window);
        }
    }
    if key_frame {
        running.capture.key_frame();
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

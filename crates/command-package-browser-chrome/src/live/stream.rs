//! One watched tab's pictures (`live-view.md` § Capture): viewers of
//! the same tab share one capture and one encoding. The capture follows the
//! tab's viewport, and its pace and encoding follow the viewer with the least
//! room.
//!
//! Nothing reaches a stream through a queue. The hub publishes who watches
//! the tab and when its layout has sized the tab; each viewer publishes its
//! latest pacing, which supersedes the one before, and wakes the stream to
//! read it.
//!
//! A capture starts only once the hub sized the tab, so it never captures at
//! a size it is about to replace, and a new size or pixel ratio starts a new
//! capture (`live-view.md` § Delivery). A resize of the tab ends its capture
//! before the page changes, and the next starts once the resize is published
//! (`CaptureGate`). Each size, viewport or scale is a new
//! generation for the viewers, which its first frame announces.

use std::{collections::HashMap, sync::Arc, time::Duration};

use tokio::{
    sync::{mpsc, watch},
    time::Instant,
};
use tokio_util::task::TaskTracker;

use crate::driver::capture::{Capture, CaptureChannel, CaptureEvent, Frame};
use crate::driver::operation::BrowserError;
use crate::live::protocol::BrowserViewport;
use crate::tabs::{tab::BrowserTab, viewport::pixels};

use crate::live::rate::{self, FRAME_RATES, SCALES};

/// Frames a viewer has not taken yet. The viewer drains them as they come
/// and drops what its page cannot take, so a full queue means the viewer
/// itself stopped.
const VIEWER_QUEUE: usize = 8;
const RETRY_LIMIT: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub(crate) enum Event {
    /// Pictures of this size in pixels follow, starting with a key frame,
    /// showing `viewport` at `scale` of its device pixels; `epoch` names this
    /// generation in the viewer's pace reports.
    Restart {
        epoch: u32,
        width: u32,
        height: u32,
        viewport: BrowserViewport,
        scale: f64,
    },
    Frame(Frame),
    /// This Host cannot capture, for this reason.
    Unavailable(String),
    /// The capture failed for this reason; the stream tries again.
    Failed(String),
    /// Captures stopped for this reason until the viewer asks again.
    Stopped(String),
}

/// What a viewer last told its tab's stream.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct Pacing {
    /// The capture epoch, the newest frame the page has or will never get,
    /// and how many frames may be in flight past it.
    pace: Option<(u32, u32, u32)>,
    /// The bit rate, frame rate and scale the viewer's path takes.
    encoding: Option<(u32, u32, f64)>,
    /// Counts the key frames the viewer asked for.
    key_frames: u64,
}

/// A viewer's side of the stream it watches.
pub(crate) struct StreamView {
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
pub(crate) struct StreamHandle {
    members: watch::Sender<Members>,
    wake: watch::Sender<u64>,
    /// Whether the hub has given the tab the size its viewers' panels ask for.
    sized: watch::Sender<bool>,
}

impl StreamHandle {
    pub fn start(tab: BrowserTab, captures: CaptureChannel, tasks: &TaskTracker) -> Self {
        let (members, watched) = watch::channel(Members::default());
        let (wake, woken) = watch::channel(0);
        let (sized, laid_out) = watch::channel(false);
        tasks.spawn(run(tab, captures, watched, woken, laid_out, tasks.clone()));
        Self {
            members,
            wake,
            sized,
        }
    }

    /// The hub sized the tab for its viewers: the capture may start.
    pub fn sized(&self) {
        self.sized.send_if_modified(|sized| !std::mem::replace(sized, true));
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
    /// The generation the viewers last heard of.
    epoch: u32,
    /// The size the capture was asked for, in pixels.
    size: (u32, u32),
    /// The size of the generation's pictures, as its first frame came; none
    /// until it comes. Chrome delivers even sides, scaling a tab with an odd
    /// one to the nearest even size of its shape, so only a frame tells it.
    shown: Option<(u32, u32)>,
    /// The viewport and the scale its pictures show.
    viewport: BrowserViewport,
    scale: f64,
    encoding: (u32, u32),
    /// The newest frame's sequence.
    sequence: u32,
}

impl Running {
    /// What tells a viewer about the generation the capture's pictures are
    /// of, once its first frame gave its size.
    fn restart(&self) -> Option<Event> {
        let (width, height) = self.shown?;
        Some(Event::Restart {
            epoch: self.epoch,
            width,
            height,
            viewport: self.viewport,
            scale: self.scale,
        })
    }
}

async fn run(
    tab: BrowserTab,
    captures: CaptureChannel,
    mut members: watch::Receiver<Members>,
    mut wake: watch::Receiver<u64>,
    mut sized: watch::Receiver<bool>,
    tasks: TaskTracker,
) {
    let mut viewers: HashMap<u64, Viewer> = HashMap::new();
    let mut viewport = tab.state().viewport.subscribe();
    let mut gate = tab.state().capture.subscribe();
    let mut running: Option<Running> = None;
    let mut epoch = 0;
    let mut retry = Duration::ZERO;
    let mut attempt = Instant::now();
    let mut unavailable: Option<String> = None;
    // Why the viewers get no picture, as they were last told.
    let mut failure: Option<String> = None;
    // Why captures stopped until a viewer asks again, while they do.
    let mut halted = captures.stopped();
    let mut stopped: Option<String> = None;
    // Ends a capture start that waits for the extension when the tab closes
    // or the stream ends.
    let stop = tab.ended().child_token();
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
        let current = viewport.borrow_and_update().current;
        let size = pixels(&current, scale);
        let ready = *sized.borrow_and_update();
        let resizing = gate.borrow_and_update().resizing();
        // The slowest viewer's budget; before any measured one, the budget a
        // sharp picture of this size needs.
        let bitrate = viewers
            .values()
            .filter_map(|viewer| viewer.bitrate)
            .min()
            .unwrap_or_else(|| rate::initial_bitrate(u64::from(size.0) * u64::from(size.1), fps));
        // A running tab capture cannot take a new size: Chrome refuses
        // `applyConstraints` that grow it ("Cannot satisfy constraints") and
        // sends no frame after ones that shrink it. So a new size, from the
        // viewers' scale, starts a new capture. A resize of the tab ends the
        // capture before it changes the page: a capture larger than its page
        // raises the page's own pixel ratio.
        if resizing || running.as_ref().is_some_and(|running| running.size != size) {
            running = None;
        }
        // A viewport changes only in a resize, which ended the capture.
        let changed = running.as_mut().filter(|running| running.scale != scale);
        if viewers.is_empty() {
            running = None;
        } else if let Some(running) = changed {
            // The same pictures at another scale, as a new generation that
            // its first frame, a key frame, announces.
            running.capture.key_frame();
            running.shown = None;
            running.scale = scale;
        } else if running.is_none()
            && ready
            && unavailable.is_none()
            && stopped.is_none()
            && Instant::now() >= attempt
            // A resize that began since `resizing` was read wakes the loop.
            && let Some(hold) = tab.state().capture.capture()
        {
            match captures
                .start(tab.target_id(), size.0, size.1, fps, bitrate, hold, &stop)
                .await
            {
                Ok(capture) => {
                    running = Some(Running {
                        capture,
                        epoch,
                        size,
                        shown: None,
                        viewport: current,
                        scale,
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
                // The channel says when captures may start again.
                Err(BrowserError::CaptureStopped(reason)) => {
                    tracing::warn!("live view capture of {} stopped: {reason}", tab.id());
                    for viewer in viewers.values() {
                        let _behind = viewer.events.try_send(Event::Stopped(reason.clone()));
                    }
                    failure = None;
                    stopped = Some(reason);
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
        // Only a start that waits for its retry time sleeps: until the hub
        // sized the tab, or while it resizes, the loop waits for them.
        let waiting = running.is_none()
            && ready
            && !resizing
            && unavailable.is_none()
            && stopped.is_none()
            && !viewers.is_empty();
        let event = async {
            match running.as_mut() {
                Some(running) => running.capture.events.recv().await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            _ = tab.ended().cancelled() => return,
            _ = viewport.changed() => {}
            // The gate lives as long as the tab this stream holds.
            _ = gate.changed() => {}
            // The hub drops a stream nobody watches.
            laid_out = sized.changed(), if !ready => if laid_out.is_err() {
                return;
            },
            _ = tokio::time::sleep_until(attempt), if waiting => {}
            // A viewer asked again, or the extension came back by itself.
            resumed = halted.changed(), if stopped.is_some() => {
                // The channel ends with the environment, as the tab does.
                if resumed.is_err() {
                    return;
                }
                if halted.borrow_and_update().is_none() {
                    stopped = None;
                    retry = Duration::ZERO;
                    attempt = Instant::now();
                }
            }
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
                    if let Some(reason) = &stopped {
                        let _behind = member.events.try_send(Event::Stopped(reason.clone()));
                    }
                    if let Some(running) = &running {
                        if let Some(restart) = running.restart() {
                            let _behind = member.events.try_send(restart);
                        }
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
                        // The first key frame of a generation, or of a size
                        // Chrome changed by itself, announces its size; the
                        // pictures before it belong to the generation before.
                        let sides = (u32::from(frame.width), u32::from(frame.height));
                        if running.shown != Some(sides) {
                            if !frame.key {
                                continue;
                            }
                            epoch += 1;
                            running.epoch = epoch;
                            running.shown = Some(sides);
                            let restart = running.restart().expect("the generation has its size");
                            for viewer in viewers.values_mut() {
                                viewer.floor = running.sequence;
                                let _behind = viewer.events.try_send(restart.clone());
                            }
                        }
                        running.sequence = frame.sequence;
                    }
                    failure = None;
                    for viewer in viewers.values() {
                        let _behind = viewer.events.try_send(Event::Frame(frame.clone()));
                    }
                }
                // A page that stopped painting before capture began sends no
                // picture; a screenshot from its surface paints one. A capture
                // that starts asks for it at once, rather than after the
                // silence that tells the extension a page is still; one still
                // silent then asks again.
                Some(CaptureEvent::Started | CaptureEvent::Stalled) => repaint(&tab, &tasks),
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

/// Paints the tab's page once, beside the stream: a still page sends a new
/// capture no picture until it paints. The paint ends with the tab, and
/// waits for a page that does not paint at most five seconds.
fn repaint(tab: &BrowserTab, tasks: &TaskTracker) {
    let page = tab.page().clone();
    let ended = tab.ended().clone();
    tasks.spawn(async move {
        tokio::select! {
            _ = ended.cancelled() => {}
            _painted = tokio::time::timeout(
                Duration::from_secs(5),
                crate::tabs::viewport::paint(&page),
            ) => {}
        }
    });
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

//! One watched source's pictures: viewers of the same source share one capture
//! and one encoding. The capture follows the source's size, and its pace and
//! encoding follow the viewer with the least room.
//!
//! Nothing reaches a stream through a queue. Its owner publishes who watches
//! the source and when the source is ready to be captured. Each viewer
//! publishes its latest pacing, which supersedes the one before, and wakes the
//! stream to read it.
//!
//! # Captures and key frames
//!
//! - A capture starts only once the owner says the source is ready, so it never
//!   captures at a size it is about to replace. A new size starts a new
//!   capture.
//! - Each size, layout or scale is a new generation for the viewers, which its
//!   first frame announces.
//! - A source is captured only while someone watches it.
//! - Key frames are sent only when needed (a new viewer, a changed source or
//!   size, or the page failing to decode), at most two a second, never on a
//!   timer.

use std::{collections::HashMap, future::Future, sync::Arc, time::Duration};

use bytes::Bytes;
use tokio::{
    sync::{mpsc, watch},
    time::Instant,
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use crate::rate::{FRAME_RATES, INITIAL_BITRATE, SCALES};

/// Frames a viewer has not taken yet. The viewer drains them as they come
/// and drops what its page cannot take, so a full queue means the viewer
/// itself stopped.
const VIEWER_QUEUE: usize = 8;
const RETRY_LIMIT: Duration = Duration::from_secs(5);
/// The least time between two key frames the viewers asked for: at most two
/// in a second.
const KEY_FRAME_SPACING: Duration = Duration::from_millis(500);

/// One encoded picture of a source: H.264 Annex B in
/// [`VIDEO_CODEC`](demi_live_view_protocol::VIDEO_CODEC), with its size in
/// pixels and whether it is a key frame.
#[derive(Debug, Clone)]
pub struct EncodedFrame {
    pub sequence: u32,
    pub key: bool,
    /// Microseconds on the source's clock.
    pub timestamp: f64,
    pub width: u16,
    pub height: u16,
    pub data: Bytes,
}

/// What a running capture reports.
#[derive(Debug)]
pub enum CaptureEvent {
    Started,
    Frame(EncodedFrame),
    /// No picture arrived: the source stopped drawing before capture began.
    Stalled,
    Failed(String),
}

/// A running capture of a source; dropping it stops the capture.
pub trait Capture: Send + 'static {
    /// The capture's next event; none once it ended.
    fn next(&mut self) -> impl Future<Output = Option<CaptureEvent>> + Send;
    /// The viewers' windows have room for `window` more frames past
    /// `sequence`, the newest they had: the capture encodes the newest
    /// picture only while it has sent fewer than that past it.
    fn ack(&self, sequence: u32, window: u32);
    fn key_frame(&self);
    fn encoding(&self, bitrate: u32, fps: u32);
}

/// Why a capture did not start.
#[derive(Debug)]
pub enum StartError {
    /// This Host cannot capture the source, for this reason; waiting
    /// changes nothing.
    Unavailable(String),
    /// Captures stopped for this reason until a viewer asks again, which
    /// [`Source::changed`] tells.
    Stopped(String),
    /// This attempt failed; the stream tries again.
    Failed(String),
}

/// What [`Source::changed`] found.
pub enum Change {
    /// What the source shows, or whether it may be captured, may have
    /// changed.
    Layout,
    /// Captures stopped by [`StartError::Stopped`] may start again.
    Resumed,
    /// The source is gone.
    Gone,
}

/// A source of pictures: a browser tab, a simulator. The stream asks it for
/// its size and starts and steers its captures; the source decides what its
/// size and readiness are.
pub trait Source: Send + 'static {
    type Capture: Capture;
    /// What a generation's pictures show besides their size, which viewers
    /// hear with each generation, such as a browser tab's viewport.
    type Layout: Clone + PartialEq + Send + Sync + 'static;

    /// Names the source in the Host's log.
    fn name(&self) -> String;
    /// Cancelled once the source is gone; the stream ends with it.
    fn ended(&self) -> CancellationToken;
    /// What the source shows now, and the size in pixels its pictures take
    /// at `scale`.
    fn layout(&mut self, scale: f64) -> (Self::Layout, (u32, u32));
    /// Whether a capture may run now; a running one ends while it may not,
    /// as while a browser tab resizes.
    fn capturable(&mut self) -> bool;
    /// Resolves once what `layout` or `capturable` answer may have changed,
    /// or, while captures are `stopped`, once they may start again.
    fn changed(&mut self, stopped: bool) -> impl Future<Output = Change> + Send;
    /// Starts a capture at `size`. None when the source cannot be captured
    /// after all, which a change then tells. `stop` ends a start that waits.
    fn start(
        &mut self,
        size: (u32, u32),
        fps: u32,
        bitrate: u32,
        stop: &CancellationToken,
    ) -> impl Future<Output = Result<Option<Self::Capture>, StartError>> + Send;
    /// A capture started, or stalled without a picture: a source that draws
    /// nothing new would send none, so it draws itself once.
    fn redraw(&self);
}

/// What a viewer hears of the source it watches.
#[derive(Clone)]
pub enum Event<L> {
    /// Pictures of this size in pixels follow, starting with a key frame,
    /// showing `layout` at `scale` of its pixels; `epoch` names this
    /// generation in the viewer's pace reports.
    Restart {
        epoch: u32,
        width: u32,
        height: u32,
        layout: L,
        scale: f64,
        /// The capture's newest frame before the viewer's first: the
        /// viewer's pictures follow it.
        sequence: u32,
    },
    Frame(EncodedFrame),
    /// This Host cannot capture, for this reason.
    Unavailable(String),
    /// The capture failed for this reason; the stream tries again.
    Failed(String),
    /// Captures stopped for this reason until the viewer asks again.
    Stopped(String),
}

/// What a viewer last told its source's stream.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Pacing {
    /// The capture epoch, the newest frame the page has or will never get,
    /// and how many frames may be in flight past it.
    pace: Option<(u32, u32, u32)>,
    /// The bit rate, frame rate and scale the viewer's path takes.
    encoding: Option<(u32, u32, f64)>,
    /// Counts the key frames the viewer asked for.
    key_frames: u64,
}

/// A viewer's side of the stream it watches.
pub struct StreamView<L> {
    pub events: mpsc::Receiver<Event<L>>,
    pacing: watch::Sender<Pacing>,
    /// Wakes the stream to read its viewers' pacing.
    wake: watch::Sender<u64>,
}

impl<L> StreamView<L> {
    /// The viewer had the frames up to `floor`, and its window has room for
    /// `window` more.
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
struct Member<L> {
    events: mpsc::Sender<Event<L>>,
    pacing: watch::Receiver<Pacing>,
}

impl<L> Clone for Member<L> {
    fn clone(&self) -> Self {
        Self {
            events: self.events.clone(),
            pacing: self.pacing.clone(),
        }
    }
}

type Members<L> = Arc<HashMap<u64, Member<L>>>;

/// The owner's side of one watched source's stream; the stream ends when
/// this is dropped or the source is gone.
pub struct StreamHandle<L> {
    members: watch::Sender<Members<L>>,
    wake: watch::Sender<u64>,
    /// Whether the owner made the source ready for its viewers, as a hub
    /// sizes a browser tab for their panels.
    ready: watch::Sender<bool>,
}

impl<L: Clone + Send + Sync + 'static> StreamHandle<L> {
    pub fn start<S: Source<Layout = L>>(source: S, tasks: &TaskTracker) -> Self {
        let (members, watched) = watch::channel(Members::default());
        let (wake, woken) = watch::channel(0);
        let (ready, readied) = watch::channel(false);
        tasks.spawn(run(source, watched, woken, readied));
        Self {
            members,
            wake,
            ready,
        }
    }

    /// The source is ready for its viewers: the capture may start.
    pub fn ready(&self) {
        self.ready.send_if_modified(|ready| !std::mem::replace(ready, true));
    }

    /// Adds viewer `id`; its view carries the pictures and its pacing.
    pub fn join(&self, id: u64) -> StreamView<L> {
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

struct Viewer<L> {
    events: mpsc::Sender<Event<L>>,
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

struct Running<C, L> {
    capture: C,
    /// The generation the viewers last heard of.
    epoch: u32,
    /// The size the capture was asked for, in pixels.
    size: (u32, u32),
    /// The size of the generation's pictures, as its first frame came; none
    /// until it comes. A source may deliver another size than it was asked
    /// for, as Chrome scales a tab with an odd side to the nearest even size
    /// of its shape, so only a frame tells it.
    shown: Option<(u32, u32)>,
    /// The layout and the scale its pictures show.
    layout: L,
    scale: f64,
    encoding: (u32, u32),
    /// The newest frame's sequence.
    sequence: u32,
}

impl<C, L: Clone> Running<C, L> {
    /// What tells a viewer about the generation the capture's pictures are
    /// of, once its first frame gave its size.
    fn restart(&self) -> Option<Event<L>> {
        let (width, height) = self.shown?;
        Some(Event::Restart {
            epoch: self.epoch,
            width,
            height,
            layout: self.layout.clone(),
            scale: self.scale,
            sequence: self.sequence,
        })
    }
}

async fn run<S: Source>(
    mut source: S,
    mut members: watch::Receiver<Members<S::Layout>>,
    mut wake: watch::Receiver<u64>,
    mut ready: watch::Receiver<bool>,
) {
    let mut viewers: HashMap<u64, Viewer<S::Layout>> = HashMap::new();
    let mut running: Option<Running<S::Capture, S::Layout>> = None;
    let mut epoch = 0;
    let mut key_frames = KeyFrames::default();
    let mut retry = Duration::ZERO;
    let mut attempt = Instant::now();
    let mut unavailable: Option<String> = None;
    // Why the viewers get no picture, as they were last told.
    let mut failure: Option<String> = None;
    // Why captures stopped until a viewer asks again, while they do.
    let mut stopped: Option<String> = None;
    let ended = source.ended();
    // Ends a capture start that waits when the source is gone or the stream
    // ends.
    let stop = ended.child_token();
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
        let (layout, size) = source.layout(scale);
        let laid_out = *ready.borrow_and_update();
        let capturable = source.capturable();
        // The slowest viewer's target; before any, where every estimate starts.
        let bitrate = viewers
            .values()
            .filter_map(|viewer| viewer.bitrate)
            .min()
            .unwrap_or(INITIAL_BITRATE);
        // A running capture does not take a new size: Chrome's tab capture
        // refuses constraints that grow it ("Cannot satisfy constraints")
        // and sends no frame after ones that shrink it. So a new size, from
        // the source or the viewers' scale, starts a new capture, as a
        // source that may not be captured ends it.
        if !capturable || running.as_ref().is_some_and(|running| running.size != size) {
            running = None;
        }
        let changed = running
            .as_mut()
            .filter(|running| running.scale != scale || running.layout != layout);
        if viewers.is_empty() {
            running = None;
        } else if let Some(running) = changed {
            // The same capture showing another layout, such as a square
            // screen turned, or at another scale: a new generation that its
            // first frame, a key frame, announces, to viewers who join later
            // too.
            running.capture.key_frame();
            running.shown = None;
            running.scale = scale;
            running.layout = layout;
        } else if running.is_none()
            && laid_out
            && capturable
            && unavailable.is_none()
            && stopped.is_none()
            && Instant::now() >= attempt
        {
            match source.start(size, fps, bitrate, &stop).await {
                Ok(Some(capture)) => {
                    running = Some(Running {
                        capture,
                        epoch,
                        size,
                        shown: None,
                        layout,
                        scale,
                        encoding: (bitrate, fps),
                        sequence: 0,
                    });
                }
                // The source changed meanwhile, which wakes the loop.
                Ok(None) => {}
                // Waiting changes nothing on a Host that cannot capture.
                Err(StartError::Unavailable(reason)) => {
                    for viewer in viewers.values() {
                        let _behind = viewer.events.try_send(Event::Unavailable(reason.clone()));
                    }
                    unavailable = Some(reason);
                }
                // The source says when captures may start again.
                Err(StartError::Stopped(reason)) => {
                    tracing::warn!("live view capture of {} stopped: {reason}", source.name());
                    for viewer in viewers.values() {
                        let _behind = viewer.events.try_send(Event::Stopped(reason.clone()));
                    }
                    failure = None;
                    stopped = Some(reason);
                }
                Err(StartError::Failed(reason)) => {
                    tracing::warn!("live view capture of {}: {reason}", source.name());
                    report(&viewers, &mut failure, reason);
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
        // Only a start that waits for its retry time sleeps: until the owner
        // made the source ready, or while it may not be captured, the loop
        // waits for them.
        let waiting = running.is_none()
            && laid_out
            && capturable
            && unavailable.is_none()
            && stopped.is_none()
            && !viewers.is_empty();
        let event = async {
            match running.as_mut() {
                Some(running) => running.capture.next().await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            _ = ended.cancelled() => return,
            // A viewer asked again, or the source came back by itself.
            change = source.changed(stopped.is_some()) => match change {
                Change::Gone => return,
                Change::Resumed => {
                    stopped = None;
                    retry = Duration::ZERO;
                    attempt = Instant::now();
                }
                Change::Layout => {}
            },
            // The owner drops a stream nobody watches.
            readied = ready.changed(), if !laid_out => if readied.is_err() {
                return;
            },
            _ = tokio::time::sleep_until(attempt), if waiting => {}
            changed = members.changed() => {
                // The owner drops a stream nobody watches.
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
                        window: 1,
                        bitrate: None,
                        fps: FRAME_RATES[0],
                        scale: SCALES[0],
                    });
                }
                // A viewer that came or went changes how far the capture may go.
                allow(&viewers, running.as_ref());
            }
            woken = wake.changed() => {
                if woken.is_err() {
                    return;
                }
                if paced(&mut viewers, running.as_ref()) {
                    key_frames.ask(Instant::now(), running.as_ref().map(|running| &running.capture));
                }
            }
            _ = tokio::time::sleep_until(key_frames.due()), if key_frames.deferred => {
                key_frames.ask(Instant::now(), running.as_ref().map(|running| &running.capture));
            }
            event = event => match event {
                Some(CaptureEvent::Frame(frame)) => {
                    // A start that was enqueued does not mean the source
                    // acquired its stream; a frame does.
                    retry = Duration::ZERO;
                    if let Some(running) = &mut running {
                        // The first key frame of a generation, or of a size
                        // the source changed by itself, announces its size;
                        // the pictures before it belong to the generation
                        // before.
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
                // A source that stopped drawing before capture began sends
                // no picture. A capture that starts asks it to draw at once,
                // rather than after the silence that tells a still source;
                // one still silent then asks again.
                Some(CaptureEvent::Started | CaptureEvent::Stalled) => source.redraw(),
                Some(CaptureEvent::Failed(message)) => {
                    tracing::warn!("live view capture of {}: {message}", source.name());
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

/// The key frames the viewers ask for after a frame they could not decode,
/// at most one per [`KEY_FRAME_SPACING`]: a request that comes sooner waits
/// until then, joined by any that come meanwhile.
#[derive(Default)]
struct KeyFrames {
    last: Option<Instant>,
    deferred: bool,
}

impl KeyFrames {
    fn due(&self) -> Instant {
        self.last.map_or_else(Instant::now, |last| last + KEY_FRAME_SPACING)
    }

    /// A viewer asked at `now`: `capture` sends a key frame now or once the
    /// spacing passed. Without a capture there is none to ask; the next
    /// capture starts with a key frame.
    fn ask<C: Capture>(&mut self, now: Instant, capture: Option<&C>) {
        let Some(capture) = capture else {
            self.deferred = false;
            return;
        };
        if self.last.is_some_and(|last| now < last + KEY_FRAME_SPACING) {
            self.deferred = true;
            return;
        }
        self.last = Some(now);
        self.deferred = false;
        capture.key_frame();
    }
}

/// Takes in what the viewers told the stream since it last looked: the latest
/// pace and encoding of each; answers whether one asked for a key frame.
fn paced<C: Capture, L>(viewers: &mut HashMap<u64, Viewer<L>>, running: Option<&Running<C, L>>) -> bool {
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
    if acknowledged {
        allow(viewers, running);
    }
    key_frame
}

/// Tells the capture how far its viewers let it go: past the least of their
/// frames, as many as the least of their windows has room for.
fn allow<C: Capture, L>(viewers: &HashMap<u64, Viewer<L>>, running: Option<&Running<C, L>>) {
    let Some(running) = running else {
        return;
    };
    let floor = viewers.values().map(|viewer| viewer.floor).min();
    let window = viewers.values().map(|viewer| viewer.window).min();
    if let (Some(floor), Some(window)) = (floor, window) {
        running.capture.ack(floor, window);
    }
}

/// Tells the viewers why they get no picture, once per reason: the capture
/// retries, and a repeated failure is not news.
fn report<L>(viewers: &HashMap<u64, Viewer<L>>, failure: &mut Option<String>, reason: String) {
    if failure.as_ref() == Some(&reason) {
        return;
    }
    for viewer in viewers.values() {
        let _behind = viewer.events.try_send(Event::Failed(reason.clone()));
    }
    *failure = Some(reason);
}

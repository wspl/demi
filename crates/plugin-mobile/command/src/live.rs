//! The simulators' live view on the Host: one invocation of `mobile.live` per
//! device a page shows.
//!
//! The stream's only state is whether it is looked at. While it is, the
//! viewer joins the device's capture and the view sends the device's
//! pictures, its state, the agent's activity and its frame; while it is not,
//! the viewer leaves the capture and the view sends nothing, not even a
//! heartbeat, until it is looked at again. A frame is drawn in a task of its
//! own, so nothing waits for it, and one the view no longer needs is not
//! finished; it names the screen it was drawn for, its orientation and size,
//! as the driver read them to draw it, so the page pairs it with pictures of
//! that orientation only. The capture starts once the view said how large it
//! shows the screen, or before, once it said the area it lays the device out
//! in, which bounds it: never waiting for the frame.
//!
//! The view tells the page the device's state when the stream is looked at
//! and again as that device changes: its screen turns, a recording starts or
//! stops. Another device's changes are not its own.
//!
//! A device is a source of the shared live view:
//!
//! - Viewers of one device share its capture.
//! - Its pictures are the screen turned upright: its lit display's, or, on a
//!   stream whose hello names one, a hinged foldable's outer or inner
//!   display's, lit or not, as its 3D view shows each on its own face. Each
//!   display is a source of its own, `m1.outer` and `m1.inner` beside the lit
//!   display's `m1`, with its own capture.
//! - The viewer's input reaches it as touches and keys in the order it arrives,
//!   beside the agent's, whose activity the view shows. A move of the hinge
//!   goes at once, outside that order, so it turns a fold under way.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use tokio::sync::{mpsc, watch};
use tokio_util::{
    sync::CancellationToken,
    task::{AbortOnDropHandle, TaskTracker},
};

use demi_command_package_mobile_protocol::{
    live::{INPUT_FAILED, LiveCorners, LiveDevice, LiveFold, LivePoints, NO_FOCUSED_FIELD, LiveModuleMessage, LiveViewerMessage},
    mobile::{FoldDisplay, Orientation},
};
use demi_command_protocol::{CommandError, Completion};
use demi_command_sdk::{InvocationContext, ResidentError};
use demi_live_view::{
    delivery::Delivery,
    frames::Reader,
    input::{self, Deliver, Input, Item},
    stream::{Capture, CaptureEvent, Change, Event, Source, StartError, StreamHandle, StreamView},
    writer::Writer,
};
use demi_live_view_protocol::{
    ACK_FRAME, CONTROL_FRAME, ChunkAck, KeyAction, LiveInputs, LiveKeyframe, LiveRelease, PointerAction,
    PointerKind, SourceId, TouchAction, ViewerInput,
};

use crate::{
    error::MobileError,
    devices::{Devices, OpenDevice},
    driver::{DeviceCapture, Folded, FrameArea, Framed, KeyPress, Points, Screen, TouchPhase},
    keys,
};

/// How long the writer may take to deliver what is queued when a view ends.
const FLUSH: Duration = Duration::from_secs(2);

/// What a generation's pictures show besides their size: which way the
/// device is turned and its screen's points.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub orientation: Orientation,
    pub points: Points,
}

/// The captures of the Host's looked-at devices, one per device and display
/// a view shows, shared by its viewers: a device's lit display, and a hinged
/// foldable's outer and inner displays that its 3D view shows on their
/// faces.
#[derive(Default)]
pub struct Hub {
    streams: Mutex<HashMap<(String, Option<FoldDisplay>), (Arc<StreamHandle<Layout>>, Fit)>>,
    viewers: AtomicU64,
    tasks: TaskTracker,
}

/// The share of a device's screen pixels each of its viewers shows it at, at
/// most all of them. Its pictures take the largest, so each viewer's panel
/// shows them at its own pixels or scaled down from a larger panel's; none
/// until a viewer says. A view says its size once it settled, so a drag of
/// the panel's edge restarts no capture at every step.
type Fit = Arc<watch::Sender<HashMap<u64, f64>>>;

/// The share of a screen of `pixels` that a view showing it at `shown`
/// device pixels needs, by the long sides, so a turn changes nothing.
fn fit(shown: (u32, u32), pixels: (u32, u32)) -> f64 {
    let long = |(width, height): (u32, u32)| f64::from(width.max(height).max(1));
    (long(shown) / long(pixels)).min(1.0)
}

/// The most of a screen of `pixels` a view laying the device out in `area`
/// can show: the screen fitted into the area, turned either way, as a
/// share of its pixels, at most all of them.
fn bound(area: FrameArea, pixels: (u32, u32)) -> f64 {
    let (width, height) = (f64::from(area.width), f64::from(area.height));
    let (across, down) = (f64::from(pixels.0.max(1)), f64::from(pixels.1.max(1)));
    let upright = (width / across).min(height / down);
    let turned = (width / down).min(height / across);
    upright.max(turned).min(1.0)
}

/// Viewer `viewer` shows the screen at `share` of its pixels.
fn show(fit: &Fit, viewer: u64, share: f64) {
    fit.send_if_modified(|shares| {
        shares.insert(viewer, share) != Some(share)
    });
}

impl Hub {
    /// Viewer `viewer`'s view of the pictures `shows` of `open`.
    fn join(
        &self,
        open: &Arc<OpenDevice>,
        shows: &Shows,
        viewer: u64,
    ) -> (Arc<StreamHandle<Layout>>, Fit, StreamView<Layout>) {
        let mut streams = self.streams.lock().expect("the streams are intact");
        let (handle, fit) = streams
            .entry((open.native_id.clone(), shows.display))
            .or_insert_with(|| {
                let fit = Arc::new(watch::Sender::new(HashMap::new()));
                let handle = StreamHandle::start(
                    DeviceSource {
                        open: open.clone(),
                        shows: shows.clone(),
                        fit: fit.clone(),
                    },
                    &self.tasks,
                );
                // A simulator's screen needs no sizing for its viewers.
                handle.ready();
                (Arc::new(handle), fit)
            })
            .clone();
        let view = handle.join(viewer);
        (handle, fit, view)
    }

    /// Viewer `viewer` stops watching `open`: a device nobody watches is no
    /// longer captured.
    fn leave(&self, open: &OpenDevice, shows: &Shows, handle: &StreamHandle<Layout>, viewer: u64) {
        handle.leave(viewer);
        let mut streams = self.streams.lock().expect("the streams are intact");
        if handle.is_empty() {
            streams.remove(&(open.native_id.clone(), shows.display));
        }
    }

    pub async fn close(&self) {
        self.streams.lock().expect("the streams are intact").clear();
        self.tasks.close();
        self.tasks.wait().await;
    }
}

/// What of a device a view's stream shows: its lit display, or a hinged
/// foldable's `display`, with that display's screen.
#[derive(Clone)]
struct Shows {
    display: Option<FoldDisplay>,
    screen: watch::Receiver<Screen>,
}

impl Shows {
    /// The pictures of `open` a hello asked for; none of a display a device
    /// that does not fold on a hinge lacks.
    fn of(open: &OpenDevice, display: Option<FoldDisplay>) -> Option<Self> {
        let screen = match display {
            Some(display) => open.device.display_screen(display)?,
            None => open.device.screen(),
        };
        Some(Self { display, screen })
    }

    /// The source its pictures name: the device's handle, and the display's
    /// name after it, as `m1.inner`.
    fn source(&self, open: &OpenDevice) -> SourceId {
        let name = match self.display {
            Some(display) => format!("{}.{display}", open.handle),
            None => open.handle.to_string(),
        };
        SourceId::try_from(name.as_str()).expect("a handle and a display fit a picture's source")
    }
}

/// A device's display as a source of pictures.
struct DeviceSource {
    open: Arc<OpenDevice>,
    shows: Shows,
    fit: Fit,
}

struct DeviceCaptureOf(Box<dyn DeviceCapture>);

impl Capture for DeviceCaptureOf {
    async fn next(&mut self) -> Option<CaptureEvent> {
        self.0.next().await
    }

    fn ack(&self, sequence: u32, window: u32) {
        self.0.ack(sequence, window);
    }

    fn key_frame(&self) {
        self.0.key_frame();
    }

    fn encoding(&self, bitrate: u32, fps: u32) {
        self.0.encoding(bitrate, fps);
    }

    fn repeat(&self) {
        self.0.repeat();
    }
}

/// `side` scaled, made even, which H.264's chroma needs.
fn even(side: u32, scale: f64) -> u32 {
    let scaled = (f64::from(side) * scale).round() as u32;
    (scaled & !1).max(2)
}

impl Source for DeviceSource {
    type Capture = DeviceCaptureOf;
    type Layout = Layout;

    fn name(&self) -> String {
        format!("{} ({})", self.shows.source(&self.open).as_str(), self.open.name)
    }

    fn ended(&self) -> CancellationToken {
        self.open.closed.clone()
    }

    fn layout(&mut self, scale: f64) -> (Layout, (u32, u32)) {
        let screen = *self.shows.screen.borrow();
        let (width, height) = screen.pixels;
        // Scaled to the largest panel's size before it is encoded.
        let share = self.fit.borrow().values().copied().reduce(f64::max).unwrap_or(1.0);
        let scale = scale * share;
        (
            Layout {
                orientation: screen.orientation,
                points: screen.points,
            },
            (even(width, scale), even(height, scale)),
        )
    }

    /// Not before a viewer said the size it shows the screen at, so no
    /// capture starts at a size it is about to replace.
    fn capturable(&mut self) -> bool {
        !self.fit.borrow().is_empty()
    }

    fn changed(&mut self, _stopped: bool) -> impl std::future::Future<Output = Change> + Send {
        let mut screen = self.shows.screen.clone();
        let mut fit = self.fit.subscribe();
        let closed = self.open.closed.clone();
        async move {
            screen.borrow_and_update();
            fit.borrow_and_update();
            tokio::select! {
                _ = closed.cancelled() => Change::Gone,
                changed = screen.changed() => match changed {
                    Ok(()) => Change::Layout,
                    Err(_) => Change::Gone,
                },
                // The sender lives as long as the source.
                _ = fit.changed() => Change::Layout,
            }
        }
    }

    fn start(
        &mut self,
        size: (u32, u32),
        fps: u32,
        bitrate: u32,
        stop: &CancellationToken,
    ) -> impl std::future::Future<Output = Result<Option<DeviceCaptureOf>, StartError>> + Send {
        let device = self.open.device.clone();
        let display = self.shows.display;
        let stop = stop.clone();
        async move {
            tokio::select! {
                _ = stop.cancelled() => Ok(None),
                started = device.capture(size, fps, bitrate, display) => match started {
                    Ok(capture) => Ok(Some(DeviceCaptureOf(capture))),
                    Err(error) => Err(StartError::Failed(error.to_string())),
                },
            }
        }
    }

    // A simulator's capture sends the screen as it is when it starts.
    fn redraw(&self) {}
}

/// What a viewer holds down on a device.
#[derive(Default)]
struct Held {
    /// The finger on the screen, where it is.
    finger: Option<Points>,
    keys: Vec<u32>,
    /// The keys down whose character went in as text, by their code: their
    /// release has nothing to release.
    typed: Vec<String>,
}

/// Delivers one viewer's input to the device it watches.
struct DeviceInput {
    notices: Writer<LiveModuleMessage>,
}

impl DeviceInput {
    /// Tells the viewer that its input did not arrive: text that found no
    /// field to go into quoting the viewer's own `text` whole, which the
    /// page shows in words of its own.
    async fn failed(&self, error: crate::driver::DriverError, text: Option<&str>) {
        match (error, text) {
            (crate::driver::DriverError::NoFocusedField(_), Some(text)) => {
                self.notices.notice(NO_FOCUSED_FIELD, text).await
            }
            (error, _) => self.notices.notice(INPUT_FAILED, &error.to_string()).await,
        }
    }
}

impl Deliver for DeviceInput {
    type Message = ViewerInput;
    type Target = Arc<OpenDevice>;
    type Held = Held;
    const POINTER: PointerKind = PointerKind::Touch;

    fn merged(message: &ViewerInput, next: ViewerInput) -> Result<ViewerInput, ViewerInput> {
        match (message, next) {
            // A finger that moved again is where it moved last.
            (
                ViewerInput::Touch {
                    action: TouchAction::Move,
                    ..
                },
                next @ ViewerInput::Touch {
                    action: TouchAction::Move,
                    ..
                },
            ) => Ok(next),
            (message, next) => input::merged(message, next),
        }
    }

    fn deliver(
        &self,
        target: &Arc<OpenDevice>,
        held: &mut Held,
        message: ViewerInput,
    ) -> impl std::future::Future<Output = ()> + Send {
        let device = target.device.clone();
        let step = finger(held, &message);
        // A key that types a character, in whatever layout the viewer's
        // keyboard has, enters that character, as the viewer's text does;
        // the others, Enter, the arrows, the modifiers and the shortcuts,
        // are pressed as keys.
        let character = match &message {
            ViewerInput::Key {
                action: KeyAction::Down,
                code,
                text: Some(text),
                ..
            } => {
                held.typed.push(code.clone());
                Some(text.clone())
            }
            ViewerInput::Key {
                action: KeyAction::Up,
                code,
                ..
            } if held.typed.contains(code) => {
                held.typed.retain(|typed| typed != code);
                return futures_util::future::Either::Left(std::future::ready(()));
            }
            _ => None,
        };
        let key = match &message {
            ViewerInput::Key { action, code, .. } if character.is_none() => keys::usage(code).map(|usage| KeyPress {
                usage,
                down: *action == KeyAction::Down,
            }),
            _ => None,
        };
        if let Some(press) = key {
            if press.down {
                held.keys.push(press.usage);
            } else {
                held.keys.retain(|usage| *usage != press.usage);
            }
        }
        let scroll = match &message {
            ViewerInput::Wheel { x, y, delta_x, delta_y, .. } if held.finger.is_none() => {
                scroll(*device.screen().borrow(), Points { x: *x, y: *y }, *delta_x, *delta_y)
            }
            _ => None,
        };
        let text = match message {
            ViewerInput::Text { text } => Some(text),
            _ => character,
        };
        futures_util::future::Either::Right(async move {
            let delivered = if let Some((phase, at)) = step {
                device.touch(phase, at).await
            } else if let Some((from, to)) = scroll {
                device.swipe(from, to).await
            } else if let Some(press) = key {
                device.key(press).await
            } else if let Some(text) = &text {
                keys::enter(&device, text).await
            } else {
                Ok(())
            };
            if let Err(error) = delivered {
                self.failed(error, text.as_deref()).await;
            }
        })
    }

    fn release(
        &self,
        target: Option<&Arc<OpenDevice>>,
        held: Held,
    ) -> impl std::future::Future<Output = ()> + Send {
        let device = target.map(|open| open.device.clone());
        async move {
            let Some(device) = device else { return };
            if let Some(at) = held.finger {
                // A finger lifted from a device that ended is lifted anyway.
                let _lifted = device.touch(TouchPhase::Up, at).await;
            }
            for usage in held.keys {
                let _up = device.key(KeyPress { usage, down: false }).await;
            }
        }
    }
}

/// The finger's step a touch or a mouse's press, drag and release makes, noting
/// where it is in `held`: the first contact is the finger, as a simulator takes
/// one.
///
/// A wheel turn at `at` is the swipe that scrolls the screen as far: the finger
/// moves against the turn, as a page follows a wheel, and stops at the screen's
/// edge. A touch screen has no wheel of its own.
fn scroll(screen: Screen, at: Points, delta_x: f64, delta_y: f64) -> Option<(Points, Points)> {
    let to = Points {
        x: (at.x - delta_x).clamp(0.0, screen.points.x),
        y: (at.y - delta_y).clamp(0.0, screen.points.y),
    };
    (to != at).then_some((at, to))
}

fn finger(held: &mut Held, message: &ViewerInput) -> Option<(TouchPhase, Points)> {
    let (phase, at) = match message {
        ViewerInput::Touch { action, contacts } => {
            let at = contacts.first().map(|contact| Points {
                x: contact.x,
                y: contact.y,
            });
            match action {
                TouchAction::Start => (TouchPhase::Down, at?),
                TouchAction::Move => (TouchPhase::Move, at?),
                TouchAction::End | TouchAction::Cancel => (TouchPhase::Up, at.or(held.finger)?),
            }
        }
        ViewerInput::Pointer {
            action, x, y, buttons, ..
        } => {
            let at = Points { x: *x, y: *y };
            match action {
                PointerAction::Down => (TouchPhase::Down, at),
                PointerAction::Move if *buttons & 1 == 1 && held.finger.is_some() => (TouchPhase::Move, at),
                PointerAction::Up if held.finger.is_some() => (TouchPhase::Up, at),
                _ => return None,
            }
        }
        _ => return None,
    };
    held.finger = (phase != TouchPhase::Up).then_some(at);
    Some((phase, at))
}

/// The device while its stream is looked at, and what the view hears of it.
struct Watched {
    source: SourceId,
    /// The screen of the display its pictures show.
    shows: watch::Receiver<Screen>,
    /// The viewer's place in the device's capture; none while its views show
    /// none of the pictures (`LiveViewerMessage::Unshown`).
    joined: Option<Joined>,
    activity: watch::Receiver<demi_live_view_protocol::LiveActivity>,
    /// The device's screen, whose turns turn its frame.
    screen: watch::Receiver<crate::driver::Screen>,
    /// Whether the device's screen is on, which the view tells the page.
    awake: watch::Receiver<bool>,
    /// Announces a frame the device came to know, until the device says
    /// it announces none.
    reframes: Option<watch::Receiver<()>>,
}

/// A viewer's place in a device's capture, which it receives the pictures
/// of.
struct Joined {
    handle: Arc<StreamHandle<Layout>>,
    fit: Fit,
    stream: StreamView<Layout>,
}

impl Joined {
    /// Viewer `viewer` joins the capture of what `shows` shows, fitted to the
    /// size its view shows the screen at when it said one, else to the area
    /// it lays the device out in.
    fn new(
        hub: &Hub,
        open: &Arc<OpenDevice>,
        shows: &Shows,
        viewer: u64,
        delivery: &mut Delivery,
        shown: Option<(u32, u32)>,
        area: Option<FrameArea>,
    ) -> Self {
        delivery.rewatch();
        let (handle, fit, stream) = hub.join(open, shows, viewer);
        let pixels = shows.screen.borrow().pixels;
        match (shown, area) {
            (Some(shown), _) => show(&fit, viewer, self::fit(shown, pixels)),
            (None, Some(area)) => show(&fit, viewer, bound(area, pixels)),
            (None, None) => {}
        }
        if let Some((bitrate, fps, scale)) = delivery.applied() {
            stream.encoding(bitrate, fps, scale);
        }
        Self { handle, fit, stream }
    }

    /// Viewer `viewer` leaves the capture: a device nobody watches is no
    /// longer captured.
    fn leave(self, hub: &Hub, open: &OpenDevice, shows: &Shows, viewer: u64, delivery: &mut Delivery) {
        self.fit.send_modify(|shares| {
            shares.remove(&viewer);
        });
        hub.leave(open, shows, &self.handle, viewer);
        delivery.rewatch();
    }
}

/// A frame drawn for the page, in a task of its own, so neither the page's
/// messages nor the pictures wait for it: the area it was drawn for and the
/// screen it was drawn for.
struct Drawn {
    area: FrameArea,
    shown: Shown,
    message: LiveModuleMessage,
}

/// What of the screen a frame is drawn for: its orientation and its size,
/// which a foldable's other display changes.
type Shown = (Orientation, (u32, u32));

fn shown(screen: &crate::driver::Screen) -> Shown {
    (screen.orientation, screen.pixels)
}

/// Serves one device's stream of a conversation's page.
pub async fn serve(
    devices: Arc<Devices>,
    hub: Arc<Hub>,
    context: InvocationContext,
) -> Result<Completion, ResidentError> {
    let conversation = context.request.context.conversation.clone();
    let (writer, writing) = Writer::<LiveModuleMessage>::start(context.output.clone());
    // Nothing is sent before the stream is looked at.
    writer.silent(true);
    let viewer = hub.viewers.fetch_add(1, Ordering::Relaxed);
    let result = view(&devices, &hub, &conversation, viewer, &writer, context).await;
    drop(writer);
    let _flushed = tokio::time::timeout(FLUSH, writing).await;
    match result {
        Ok(()) => Ok(Completion {
            exit_code: 0,
            error: None,
        }),
        Err(None) => Err(ResidentError::Cancelled),
        Err(Some(message)) => Ok(Completion {
            exit_code: 2,
            error: Some(CommandError {
                code: "invalid_input".into(),
                message,
            }),
        }),
    }
}

/// The page's messages, read by one task: a read the loop's other events
/// interrupted would leave its demand for input open, and the next read
/// would make a second one, which the runner refuses. The channel ends with
/// the page's side, after a refusal of a frame the protocol refuses.
fn read(mut reader: Reader) -> (mpsc::Receiver<Result<FromPage, String>>, AbortOnDropHandle<()>) {
    // Bounded: a loop that stops reading holds back the page, not memory.
    let (messages, inbound) = mpsc::channel(64);
    let task = tokio::spawn(async move {
        loop {
            let delivered = match next(&mut reader).await {
                Ok(None) => break,
                Err(error) => {
                    // A view that ended meanwhile hears of nothing more.
                    let _ended = messages.send(Err(error)).await;
                    break;
                }
                Ok(Some(message)) => messages.send(Ok(message)).await.is_ok(),
            };
            if !delivered {
                break;
            }
        }
    });
    (inbound, AbortOnDropHandle::new(task))
}

/// What the page sends: control messages, and acknowledgements of the video
/// chunks.
enum FromPage {
    Message(LiveViewerMessage),
    Acks(Vec<ChunkAck>),
}

/// The page's next message; none once it ended its side, a refusal for a
/// frame the protocol refuses.
async fn next(reader: &mut Reader) -> Result<Option<FromPage>, String> {
    let Some(frame) = reader.next().await.map_err(|error| error.to_string())? else {
        return Ok(None);
    };
    match frame.kind {
        CONTROL_FRAME => demi_shared_types::decode_slice(&frame.payload)
            .map(|message| Some(FromPage::Message(message)))
            .map_err(|error| error.to_string()),
        ACK_FRAME => ChunkAck::split(&frame.payload)
            .map(|acks| Some(FromPage::Acks(acks)))
            .map_err(|error| error.to_string()),
        kind => Err(format!("the page sent a frame of kind {kind}, which it does not send")),
    }
}

/// The view itself; `Err(None)` when it was cancelled, `Err(Some(_))` when
/// the page broke the protocol.
async fn view(
    devices: &Arc<Devices>,
    hub: &Arc<Hub>,
    conversation: &str,
    viewer: u64,
    writer: &Writer<LiveModuleMessage>,
    context: InvocationContext,
) -> Result<(), Option<String>> {
    let InvocationContext {
        input: page,
        cancellation,
        ..
    } = context;
    let (mut inbound, _reading) = read(Reader::new(page));
    let (device, display) = tokio::select! {
        _ = cancellation.cancelled() => return Err(None),
        first = inbound.recv() => match first.transpose().map_err(Some)? {
            Some(FromPage::Message(LiveViewerMessage::Hello { device, display, .. })) => (device, display),
            Some(_) => return Err(Some("a view starts with hello".into())),
            None => return Ok(()),
        },
    };
    let open = match devices.get(conversation, &device, &cancellation).await {
        Ok(open) => open,
        Err(MobileError::Cancelled) => return Err(None),
        // The device is not this conversation's here, or is gone.
        Err(MobileError::NotOpen(_)) => {
            writer.control(&LiveModuleMessage::Gone {}).await;
            return Ok(());
        }
        // Not taken back yet: the view ends and opens again.
        Err(error) => return Err(Some(error.to_string())),
    };
    let Some(shows) = Shows::of(&open, display) else {
        return Err(Some(format!("{} does not fold on a hinge", open.handle)));
    };
    let tasks = TaskTracker::new();
    let ended = CancellationToken::new();
    let _ended = ended.clone().drop_guard();
    let (input, input_task) = Input::start(
        &tasks,
        ended.clone(),
        DeviceInput {
            notices: writer.until(&ended),
        },
    );
    // The view's moves of the hinge, the latest only, one at a time and in
    // their order: a move that came while another went out replaces any that
    // waited, so the hinge ends at the last angle asked.
    let (hinge, mut hinges) = watch::channel::<Option<u16>>(None);
    {
        let open = open.clone();
        let notices = writer.until(&ended);
        tasks.spawn(async move {
            while hinges.changed().await.is_ok() {
                let Some(degrees) = *hinges.borrow_and_update() else { continue };
                if let Err(error) = open.device.hinge(degrees).await {
                    notices.notice(INPUT_FAILED, &error.to_string()).await;
                }
            }
        });
    }
    let mut changes = open.changes();
    let mut watched: Option<Watched> = None;
    let mut delivery = Delivery::new();
    // The size the view lays the device out at, which its frame fits, and
    // the size and orientation of the frame the view has.
    let mut area: Option<FrameArea> = None;
    let mut framed: Option<(FrameArea, Shown)> = None;
    let mut drawing: Option<AbortOnDropHandle<Drawn>> = None;
    // The size the view shows the screen at, which a capture that starts
    // when the stream is looked at again fits at once.
    let mut shown: Option<(u32, u32)> = None;
    // Whether the views show none of the pictures, until one asks for a
    // frame or says a size again.
    let mut unshown = false;
    let result = loop {
        let due = delivery.due();
        // The device's pictures and the agent's activity on it, while the
        // stream is looked at.
        let from_device = async {
            match watched.as_mut() {
                Some(watched) => tokio::select! {
                    event = async {
                        match watched.joined.as_mut() {
                            Some(joined) => joined.stream.events.recv().await,
                            None => std::future::pending().await,
                        }
                    } => FromDevice::Event(event),
                    changed = watched.activity.changed() => FromDevice::Activity(changed.is_ok()),
                    changed = watched.screen.changed() => FromDevice::Screen(changed.is_ok()),
                    changed = watched.awake.changed() => FromDevice::Power(changed.is_ok()),
                    changed = async {
                        match watched.reframes.as_mut() {
                            Some(reframes) => reframes.changed().await.is_ok(),
                            None => std::future::pending().await,
                        }
                    } => FromDevice::Reframed(changed),
                },
                None => std::future::pending().await,
            }
        };
        let drawn = async {
            match drawing.as_mut() {
                Some(task) => task.await.ok(),
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            _ = cancellation.cancelled() => break Err(None),
            () = open.closed.cancelled() => {
                // Only a device that is over is gone; one whose connection
                // was lost is reconnected to by the view's next stream.
                if open.gone.is_cancelled() {
                    writer.control(&LiveModuleMessage::Gone {}).await;
                    break Ok(());
                }
                break Err(Some("the device's connection ended".into()));
            }
            message = inbound.recv() => match message.transpose() {
                Err(error) => break Err(Some(error)),
                Ok(None) => break Ok(()),
                Ok(Some(FromPage::Acks(acks))) => {
                    delivery.acknowledged(&acks, joined_stream(&watched));
                }
                Ok(Some(FromPage::Message(message))) => match message {
                    LiveViewerMessage::Hello { .. } => break Err(Some("a view says hello once".into())),
                    LiveViewerMessage::Looked { looked } => {
                        if looked && watched.is_none() {
                            watched = Some(look(hub, &open, &shows, viewer, &input, &mut delivery, shown, area, unshown, writer).await);
                            state(&open, writer).await;
                            // A frame the view asked for while nobody looked is drawn now.
                            drawing = draw(&open, area, framed);
                        } else if !looked && watched.is_some() {
                            unlook(hub, &open, &shows, &mut watched, viewer, &input, &mut delivery, writer).await;
                            drawing = None;
                        }
                    }
                    LiveViewerMessage::Input { input: message } => {
                        if watched.is_some() {
                            input.send(message);
                        }
                    }
                    LiveViewerMessage::Keyframe(LiveKeyframe { generation }) => {
                        delivery.key_frame(generation, joined_stream(&watched));
                    }
                    LiveViewerMessage::Release(LiveRelease {}) => input.control(Item::Release).await,
                    LiveViewerMessage::Frame { width, height, scale } => {
                        let laid = FrameArea { width, height, scale };
                        area = Some(laid);
                        // The page asks for a frame it lacks, such as one it
                        // could not show: it is drawn whatever was sent before.
                        framed = None;
                        unshown = false;
                        if let Some(watched) = watched.as_mut() {
                            // A request the page made since replaces the frame under way.
                            drawing = draw(&open, area, framed);
                            match watched.joined.as_ref() {
                                // The screen shows no larger than the area, which
                                // the capture fits until the view says its size.
                                Some(joined) if shown.is_none() => {
                                    show(&joined.fit, viewer, bound(laid, shows.screen.borrow().pixels));
                                }
                                Some(_) => {}
                                None => {
                                    watched.joined = Some(Joined::new(hub, &open, &shows, viewer, &mut delivery, shown, area));
                                }
                            }
                        }
                    }
                    LiveViewerMessage::Unshown {} => {
                        shown = None;
                        area = None;
                        unshown = true;
                        if let Some(joined) = watched.as_mut().and_then(|watched| watched.joined.take()) {
                            joined.leave(hub, &open, &shows, viewer, &mut delivery);
                        }
                    }
                    // Not in the device's input queue, where a fold under way
                    // would hold it: the hinge turns from where it is.
                    LiveViewerMessage::Hinge { degrees } => {
                        hinge.send_replace(Some(degrees));
                    }
                    LiveViewerMessage::Wake {} => {
                        if watched.is_some() {
                            let open = open.clone();
                            let notices = writer.until(&ended);
                            tasks.spawn(async move {
                                // In the device's input queue, so a second Wake finds
                                // the screen the first turned on and presses nothing.
                                let _input = open.inputs.lock().await;
                                if let Err(error) = open.device.wake().await {
                                    notices.notice(INPUT_FAILED, &error.to_string()).await;
                                }
                            });
                        }
                    }
                    LiveViewerMessage::Shown { width, height } => {
                        shown = Some((width, height));
                        unshown = false;
                        if let Some(watched) = watched.as_mut() {
                            match watched.joined.as_ref() {
                                Some(joined) => {
                                    let pixels = shows.screen.borrow().pixels;
                                    show(&joined.fit, viewer, fit((width, height), pixels));
                                }
                                None => {
                                    watched.joined = Some(Joined::new(hub, &open, &shows, viewer, &mut delivery, shown, area));
                                }
                            }
                        }
                    }
                },
            },
            from_device = from_device => match from_device {
                FromDevice::Event(Some(event)) => {
                    if let Some(current) = watched.as_ref() {
                        picture(current, event, &mut delivery, writer).await;
                    }
                }
                // The device's capture ended with the device, whose close
                // ends the view, or with the resident.
                FromDevice::Event(None) => {
                    unlook(hub, &open, &shows, &mut watched, viewer, &input, &mut delivery, writer).await;
                }
                FromDevice::Activity(true) => {
                    if let Some(watched) = &watched {
                        let activity = *watched.activity.borrow();
                        writer.control(&LiveModuleMessage::Activity { activity }).await;
                    }
                }
                // The device's activity ended with it; its close says so.
                FromDevice::Activity(false) => {}
                // A turned device's frame turns with it, and the page hears
                // of its orientation.
                FromDevice::Screen(true) => {
                    drawing = draw(&open, area, framed);
                    state(&open, writer).await;
                }
                FromDevice::Screen(false) => {}
                FromDevice::Power(true) => {
                    if let Some(watched) = &watched {
                        screen_state(watched, writer).await;
                    }
                }
                // The device ended; its close says so.
                FromDevice::Power(false) => {}
                // The device came to know its frame, such as an Android
                // screen's corners read in the background.
                FromDevice::Reframed(true) => {
                    framed = None;
                    drawing = draw(&open, area, framed);
                }
                FromDevice::Reframed(false) => {
                    if let Some(watched) = watched.as_mut() {
                        watched.reframes = None;
                    }
                }
            },
            drawn = drawn => {
                drawing = None;
                // A draw that panicked leaves the view with the frame it has.
                if let Some(Drawn { area: drawn_for, shown, message }) = drawn
                    && watched.is_some()
                    && area == Some(drawn_for)
                {
                    if matches!(&message, LiveModuleMessage::Frame { frame: Some(_), .. }) {
                        framed = Some((drawn_for, shown));
                    }
                    writer.control(&message).await;
                }
            }
            changed = changes.changed() => {
                // The sender lives as long as the device.
                if changed.is_ok() && watched.is_some() && !open.closed.is_cancelled() {
                    state(&open, writer).await;
                }
            }
            _ = tokio::time::sleep_until(due.unwrap_or_else(tokio::time::Instant::now)), if due.is_some() => {
                delivery.release(writer);
            }
        }
    };
    unlook(hub, &open, &shows, &mut watched, viewer, &input, &mut delivery, writer).await;
    // The hinge's task ends with its sender, once the move under way is out.
    drop(hinge);
    drop(input);
    let _released = tokio::time::timeout(FLUSH, input_task).await;
    ended.cancel();
    tasks.close();
    tasks.wait().await;
    result
}

/// The capture `watched`'s viewer has joined, if it looks at the stream and
/// its views show the pictures.
fn joined_stream(watched: &Option<Watched>) -> Option<&StreamView<Layout>> {
    watched.as_ref().and_then(joined_stream_of)
}

fn joined_stream_of(watched: &Watched) -> Option<&StreamView<Layout>> {
    watched.joined.as_ref().map(|joined| &joined.stream)
}

/// What the device said while its stream is looked at.
enum FromDevice<E> {
    Event(Option<E>),
    /// Whether the activity changed, false once its sender is gone.
    Activity(bool),
    /// Whether the screen changed, as it turns or a foldable lights its
    /// other display, false once the device is gone.
    Screen(bool),
    /// Whether the screen turned on or off, false once the device is gone.
    Power(bool),
    /// Whether the device came to know its frame, false once it announces
    /// no more.
    Reframed(bool),
}

/// Tells the page whether the device's screen is on.
async fn screen_state(watched: &Watched, writer: &Writer<LiveModuleMessage>) {
    let on = *watched.awake.borrow();
    writer.control(&LiveModuleMessage::Screen { on }).await;
}

/// Tells the page the device as it is.
async fn state(open: &OpenDevice, writer: &Writer<LiveModuleMessage>) {
    let device = LiveDevice {
        name: open.name.clone(),
        platform: open.platform,
        os: open.os.clone(),
        model: (!open.model.is_empty()).then(|| open.model.clone()),
        recording: open.recording().is_some(),
        orientation: open.device.screen().borrow().orientation,
        fold: open.device.screen().borrow().fold.map(|fold| live_fold(fold, open.device.body())),
    };
    writer.control(&LiveModuleMessage::State { device }).await;
}

/// How the page hears of a fold.
fn live_fold(fold: Folded, body: Option<String>) -> LiveFold {
    let points = |points: Points| LivePoints {
        width: points.x,
        height: points.y,
    };
    match fold {
        Folded::Hinge {
            degrees,
            display,
            outer,
            inner,
        } => LiveFold::Hinge {
            degrees,
            display,
            outer: points(outer),
            inner: points(inner),
            body,
        },
        Folded::Twofold { open } => LiveFold::Twofold { open },
    }
}

/// The stream is looked at: the viewer joins the device's capture, fitted
/// at once to the size the view shows the screen at when it said one, else
/// to the area it lays the device out in, and the writer speaks again.
async fn look(
    hub: &Hub,
    open: &Arc<OpenDevice>,
    shows: &Shows,
    viewer: u64,
    input: &Input<DeviceInput>,
    delivery: &mut Delivery,
    shown: Option<(u32, u32)>,
    area: Option<FrameArea>,
    unshown: bool,
    writer: &Writer<LiveModuleMessage>,
) -> Watched {
    writer.silent(false);
    input.control(Item::Watch(Some(open.clone()))).await;
    delivery.rewatch();
    let joined = (!unshown).then(|| Joined::new(hub, open, shows, viewer, delivery, shown, area));
    writer
        .control(&LiveModuleMessage::Inputs {
            inputs: LiveInputs {
                pointer: DeviceInput::POINTER,
            },
        })
        .await;
    let activity = open.activity.subscribe();
    let current = *activity.borrow();
    writer.control(&LiveModuleMessage::Activity { activity: current }).await;
    let mut awake = open.device.awake();
    awake.borrow_and_update();
    let watched = Watched {
        source: shows.source(open),
        shows: shows.screen.clone(),
        joined,
        activity,
        screen: open.device.screen(),
        awake,
        reframes: Some(open.device.frame_changes()),
    };
    screen_state(&watched, writer).await;
    watched
}

/// The stream is no longer looked at: the viewer leaves the device's
/// capture, lets go what it held on the device, and the writer falls
/// silent.
async fn unlook(
    hub: &Hub,
    open: &OpenDevice,
    shows: &Shows,
    watched: &mut Option<Watched>,
    viewer: u64,
    input: &Input<DeviceInput>,
    delivery: &mut Delivery,
    writer: &Writer<LiveModuleMessage>,
) {
    let Some(previous) = watched.take() else { return };
    if let Some(joined) = previous.joined {
        joined.leave(hub, open, shows, viewer, delivery);
    }
    input.control(Item::Watch(None)).await;
    delivery.rewatch();
    writer.silent(true);
}

/// Draws the device's frame for `area` in a task, when the view does not
/// have it at that size and for the screen as it is shown yet. The frame is
/// sent for the screen the driver drew it for, which a turn or a fold
/// meanwhile may have left: the page keeps it for that orientation.
fn draw(
    open: &Arc<OpenDevice>,
    area: Option<FrameArea>,
    framed: Option<(FrameArea, Shown)>,
) -> Option<AbortOnDropHandle<Drawn>> {
    let area = area?;
    if framed == Some((area, shown(&open.device.screen().borrow()))) {
        return None;
    }
    let open = open.clone();
    Some(AbortOnDropHandle::new(tokio::spawn(async move {
        let started = std::time::Instant::now();
        let (frame, screen) = match open.device.frame(area).await {
            Ok(Some(Framed { frame, screen })) => {
                (Some(crate::frame::live(frame, open.platform, &open.model, screen.orientation)), screen)
            }
            // The view shows the device once the device announces its frame.
            Ok(None) => (None, *open.device.screen().borrow()),
            Err(error) => {
                // The drawn frame stands in for a frame the driver could not draw.
                tracing::warn!(event = "live_frame.draw_failed", name = %open.name, %error, "a frame was not drawn");
                let screen = *open.device.screen().borrow();
                let frame = crate::frame::drawn(LiveCorners::round(0.0), open.platform, &open.model, screen.orientation);
                (Some(frame), screen)
            }
        };
        let orientation = screen.orientation;
        tracing::debug!(event = "live_frame.drawn", name = %open.name, ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX), "a frame was drawn");
        Drawn {
            area,
            shown: shown(&screen),
            message: LiveModuleMessage::Frame {
                width: area.width,
                height: area.height,
                scale: area.scale,
                orientation,
                points_width: screen.points.x,
                points_height: screen.points.y,
                frame,
            },
        }
    })))
}

async fn picture(
    watched: &Watched,
    event: Event<Layout>,
    delivery: &mut Delivery,
    writer: &Writer<LiveModuleMessage>,
) {
    match event {
        Event::Restart {
            epoch,
            width,
            height,
            layout,
            scale,
            sequence,
        } => {
            let generation = delivery.restart(epoch, width, height, scale, sequence, joined_stream_of(watched));
            // The share of the screen's pixels the pictures have, the
            // panel's fit and the path's scale together.
            let pixels = watched.shows.borrow().pixels;
            let share = fit((width, height), pixels);
            writer
                .control(&LiveModuleMessage::Stream {
                    generation,
                    width,
                    height,
                    points_width: layout.points.x,
                    points_height: layout.points.y,
                    orientation: layout.orientation,
                    scale: share,
                })
                .await;
        }
        Event::Frame(frame) => delivery.frame(&watched.source, &frame, joined_stream_of(watched)),
        Event::Unavailable(reason) => {
            writer
                .notice(demi_command_package_mobile_protocol::live::CAPTURE_UNAVAILABLE, &reason)
                .await;
        }
        Event::Failed(reason) | Event::Stopped(reason) => {
            writer
                .notice(demi_command_package_mobile_protocol::live::CAPTURE_FAILED, &reason)
                .await;
        }
    }
}

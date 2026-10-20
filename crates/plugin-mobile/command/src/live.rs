//! The simulators' live view on the Host: one invocation of `mobile.live` per
//! view of a conversation's page.
//!
//! A device is a source of the shared live view:
//!
//! - Viewers of one device share its capture.
//! - Its pictures are the screen turned upright.
//! - The viewer's input reaches it as touches and keys in the order it arrives,
//!   beside the agent's, whose activity the view shows.

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
    live::{INPUT_FAILED, LiveCorners, LiveDevice, NO_FOCUSED_FIELD, LiveModuleMessage, LiveViewerMessage},
    mobile::{DeviceId, Orientation},
};
use demi_command_protocol::{CommandError, Completion};
use demi_command_sdk::{InvocationContext, ServiceError};
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
    devices::{Devices, OpenDevice, Watching},
    driver::{DeviceCapture, FrameArea, KeyPress, Points, Screen, TouchPhase},
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

/// The captures of the Host's watched devices, one per device, shared by
/// its viewers.
#[derive(Default)]
pub struct Hub {
    streams: Mutex<HashMap<String, (Arc<StreamHandle<Layout>>, Fit)>>,
    viewers: AtomicU64,
    tasks: TaskTracker,
}

/// The share of a device's screen pixels its pictures take: the largest
/// any of its viewers has shown it at since its capture began, at most
/// all of them; none until a viewer says.
type Fit = Arc<watch::Sender<Option<f64>>>;

/// The share of a screen of `pixels` that a view showing it at `shown`
/// device pixels needs, by the long sides, so a turn changes nothing.
fn fit(shown: (u32, u32), pixels: (u32, u32)) -> f64 {
    let long = |(width, height): (u32, u32)| f64::from(width.max(height).max(1));
    (long(shown) / long(pixels)).min(1.0)
}

/// Widens `fit` to `share` when it is larger: a panel that shrinks keeps
/// the pictures it has, scaled down, rather than restarting them.
fn widen(fit: &Fit, share: f64) {
    fit.send_if_modified(|current| match current {
        Some(current) if *current >= share => false,
        _ => {
            *current = Some(share);
            true
        }
    });
}

impl Hub {
    /// Viewer `viewer`'s view of `open`'s pictures.
    fn join(&self, open: &Arc<OpenDevice>, viewer: u64) -> (Arc<StreamHandle<Layout>>, Fit, StreamView<Layout>) {
        let mut streams = self.streams.lock().expect("the streams are intact");
        let (handle, fit) = streams
            .entry(open.native_id.clone())
            .or_insert_with(|| {
                let fit = Arc::new(watch::Sender::new(None));
                let handle = StreamHandle::start(
                    DeviceSource {
                        open: open.clone(),
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
    fn leave(&self, open: &OpenDevice, handle: &StreamHandle<Layout>, viewer: u64) {
        handle.leave(viewer);
        let mut streams = self.streams.lock().expect("the streams are intact");
        if handle.is_empty() {
            streams.remove(&open.native_id);
        }
    }

    pub async fn close(&self) {
        self.streams.lock().expect("the streams are intact").clear();
        self.tasks.close();
        self.tasks.wait().await;
    }
}

/// A device as a source of pictures.
struct DeviceSource {
    open: Arc<OpenDevice>,
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
        format!("{} ({})", self.open.handle, self.open.name)
    }

    fn ended(&self) -> CancellationToken {
        self.open.closed.clone()
    }

    fn layout(&mut self, scale: f64) -> (Layout, (u32, u32)) {
        let screen = *self.open.device.screen().borrow();
        let (width, height) = screen.pixels;
        // Scaled to the panel's size before it is encoded.
        let scale = scale * self.fit.borrow().unwrap_or(1.0);
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
        self.fit.borrow().is_some()
    }

    fn changed(&mut self, _stopped: bool) -> impl std::future::Future<Output = Change> + Send {
        let mut screen = self.open.device.screen();
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
        let stop = stop.clone();
        async move {
            tokio::select! {
                _ = stop.cancelled() => Ok(None),
                started = device.capture(size, fps, bitrate) => match started {
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

/// A device a viewer watches, and what it hears of it.
struct Watched {
    open: Arc<OpenDevice>,
    source: SourceId,
    handle: Arc<StreamHandle<Layout>>,
    fit: Fit,
    stream: StreamView<Layout>,
    activity: watch::Receiver<demi_live_view_protocol::LiveActivity>,
    /// The device's screen, whose turns turn its frame.
    screen: watch::Receiver<crate::driver::Screen>,
    /// Whether the device's screen is on, which the view tells the page.
    awake: watch::Receiver<bool>,
    /// The size the view lays the device out at, which its frame fits.
    area: Option<FrameArea>,
    /// The size and the orientation of the frame the view has.
    framed: Option<(FrameArea, Orientation)>,
    /// Announces a frame the device came to know, until the device says
    /// it announces none.
    reframes: Option<watch::Receiver<()>>,
    _watching: Watching,
}

/// Serves one view of `conversation`'s devices.
pub async fn serve(
    devices: Arc<Devices>,
    hub: Arc<Hub>,
    context: InvocationContext,
) -> Result<Completion, ServiceError> {
    let conversation = context.request.context.conversation.clone();
    let (writer, writing) = Writer::<LiveModuleMessage>::start(context.output.clone());
    let viewer = hub.viewers.fetch_add(1, Ordering::Relaxed);
    let result = view(&devices, &hub, &conversation, viewer, &writer, context).await;
    drop(writer);
    let _flushed = tokio::time::timeout(FLUSH, writing).await;
    match result {
        Ok(()) => Ok(Completion {
            exit_code: 0,
            error: None,
        }),
        Err(None) => Err(ServiceError::Cancelled),
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
    tokio::select! {
        _ = cancellation.cancelled() => return Err(None),
        first = inbound.recv() => match first.transpose().map_err(Some)? {
            Some(FromPage::Message(LiveViewerMessage::Hello(_))) => {}
            Some(_) => return Err(Some("a view starts with hello".into())),
            None => return Ok(()),
        },
    }
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
    let mut revisions = devices.revisions();
    let mut watched: Option<Watched> = None;
    let mut delivery = Delivery::new();
    state(devices, conversation, &watched, writer).await;
    let result = loop {
        let due = delivery.due();
        // The watched device's pictures and the agent's activity on it.
        let from_device = async {
            match watched.as_mut() {
                Some(watched) => tokio::select! {
                    event = watched.stream.events.recv() => FromDevice::Event(event),
                    changed = watched.activity.changed() => FromDevice::Activity(changed.is_ok()),
                    changed = watched.screen.changed() => FromDevice::Turned(changed.is_ok()),
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
        tokio::select! {
            _ = cancellation.cancelled() => break Err(None),
            message = inbound.recv() => match message.transpose() {
                Err(error) => break Err(Some(error)),
                Ok(None) => break Ok(()),
                Ok(Some(FromPage::Acks(acks))) => {
                    delivery.acknowledged(&acks, watched.as_ref().map(|watched| &watched.stream));
                }
                Ok(Some(FromPage::Message(message))) => match message {
                    LiveViewerMessage::Hello(_) => break Err(Some("a view says hello once".into())),
                    LiveViewerMessage::Watch { device } => {
                        let open = device.and_then(|device| devices.get(conversation, &device).ok());
                        rewatch(hub, &mut watched, open, viewer, &input, &mut delivery, writer).await;
                        state(devices, conversation, &watched, writer).await;
                    }
                    LiveViewerMessage::Input { device, input: message } => {
                        if watched.as_ref().is_some_and(|watched| watched.open.handle == device) {
                            input.send(message);
                        }
                    }
                    LiveViewerMessage::Keyframe(LiveKeyframe { generation }) => {
                        delivery.key_frame(generation, watched.as_ref().map(|watched| &watched.stream));
                    }
                    LiveViewerMessage::Release(LiveRelease {}) => input.control(Item::Release).await,
                    LiveViewerMessage::Frame { device, width, height, scale } => {
                        if let Some(watched) = watched.as_mut()
                            && watched.open.handle == device
                        {
                            watched.area = Some(FrameArea { width, height, scale });
                            frame(watched, writer).await;
                        }
                    }
                    LiveViewerMessage::Wake { device } => {
                        if let Some(watched) = watched.as_ref()
                            && watched.open.handle == device
                        {
                            let open = watched.open.clone();
                            let notices = writer.until(&ended);
                            tasks.spawn(async move {
                                // On the device's input lane, so a second Wake finds
                                // the screen the first turned on and presses nothing.
                                let _input = open.inputs.lock().await;
                                if let Err(error) = open.device.wake().await {
                                    notices.notice(INPUT_FAILED, &error.to_string()).await;
                                }
                            });
                        }
                    }
                    LiveViewerMessage::Shown { device, width, height } => {
                        if let Some(watched) = watched.as_ref()
                            && watched.open.handle == device
                        {
                            let pixels = watched.open.device.screen().borrow().pixels;
                            widen(&watched.fit, fit((width, height), pixels));
                        }
                    }
                },
            },
            from_device = from_device => match from_device {
                FromDevice::Event(Some(event)) => picture(&mut watched, event, &mut delivery, writer).await,
                // The device closed; the next state says so.
                FromDevice::Event(None) => rewatch(hub, &mut watched, None, viewer, &input, &mut delivery, writer).await,
                FromDevice::Activity(true) => {
                    if let Some(watched) = &watched {
                        let activity = *watched.activity.borrow();
                        writer.control(&LiveModuleMessage::Activity { device: watched.open.handle.clone(), activity }).await;
                    }
                }
                // The device's activity ended with it; its stream's end says so.
                FromDevice::Activity(false) => {}
                // A turned device's frame turns with it.
                FromDevice::Turned(true) => {
                    if let Some(watched) = watched.as_mut() {
                        frame(watched, writer).await;
                    }
                }
                FromDevice::Turned(false) => {}
                FromDevice::Power(true) => {
                    if let Some(watched) = &watched {
                        screen_state(watched, writer).await;
                    }
                }
                // The device ended; its stream's end says so.
                FromDevice::Power(false) => {}
                // The device came to know its frame, such as an Android
                // screen's corners read in the background.
                FromDevice::Reframed(true) => {
                    if let Some(watched) = watched.as_mut() {
                        watched.framed = None;
                        frame(watched, writer).await;
                    }
                }
                FromDevice::Reframed(false) => {
                    if let Some(watched) = watched.as_mut() {
                        watched.reframes = None;
                    }
                }
            },
            changed = revisions.changed() => {
                if changed.is_err() {
                    break Ok(());
                }
                if let Some(current) = &watched
                    && current.open.closed.is_cancelled()
                {
                    rewatch(hub, &mut watched, None, viewer, &input, &mut delivery, writer).await;
                }
                state(devices, conversation, &watched, writer).await;
            }
            _ = tokio::time::sleep_until(due.unwrap_or_else(tokio::time::Instant::now)), if due.is_some() => {
                delivery.release(writer);
            }
        }
    };
    rewatch(hub, &mut watched, None, viewer, &input, &mut delivery, writer).await;
    drop(input);
    let _released = tokio::time::timeout(FLUSH, input_task).await;
    ended.cancel();
    tasks.close();
    tasks.wait().await;
    result
}

/// What the watched device said.
enum FromDevice<E> {
    Event(Option<E>),
    /// Whether the activity changed, false once its sender is gone.
    Activity(bool),
    /// Whether the screen changed, false once the device is gone.
    Turned(bool),
    /// Whether the screen turned on or off, false once the device is gone.
    Power(bool),
    /// Whether the device came to know its frame, false once it announces
    /// no more.
    Reframed(bool),
}

/// Tells the page whether the watched device's screen is on.
async fn screen_state(watched: &Watched, writer: &Writer<LiveModuleMessage>) {
    let on = *watched.awake.borrow();
    writer
        .control(&LiveModuleMessage::Screen {
            device: watched.open.handle.clone(),
            on,
        })
        .await;
}

/// Tells the page the conversation's devices and the one it watches.
async fn state(
    devices: &Devices,
    conversation: &str,
    watched: &Option<Watched>,
    writer: &Writer<LiveModuleMessage>,
) {
    let listed = devices
        .of(conversation)
        .into_iter()
        .map(|open| LiveDevice {
            id: open.handle.clone(),
            name: open.name.clone(),
            platform: open.platform,
            os: open.os.clone(),
            model: (!open.model.is_empty()).then(|| open.model.clone()),
            recording: open.recording.lock().expect("the recording is intact").is_some(),
            orientation: open.device.screen().borrow().orientation,
        })
        .collect();
    writer
        .control(&LiveModuleMessage::State {
            devices: listed,
            watched: watched.as_ref().map(|watched| watched.open.handle.clone()),
        })
        .await;
}

/// The viewer watches `open`, or nothing: what it held on the device before
/// is let go, and the pictures start over.
async fn rewatch(
    hub: &Hub,
    watched: &mut Option<Watched>,
    open: Option<Arc<OpenDevice>>,
    viewer: u64,
    input: &Input<DeviceInput>,
    delivery: &mut Delivery,
    writer: &Writer<LiveModuleMessage>,
) {
    let same = match (watched.as_ref(), open.as_ref()) {
        (Some(current), Some(next)) => Arc::ptr_eq(&current.open, next),
        (None, None) => true,
        _ => false,
    };
    if same {
        return;
    }
    if let Some(previous) = watched.take() {
        hub.leave(&previous.open, &previous.handle, viewer);
    }
    input.control(Item::Watch(open.clone())).await;
    delivery.rewatch();
    let Some(open) = open else { return };
    let (handle, fit, stream) = hub.join(&open, viewer);
    if let Some((bitrate, fps, scale)) = delivery.applied() {
        stream.encoding(bitrate, fps, scale);
    }
    let device: DeviceId = open.handle.clone();
    writer
        .control(&LiveModuleMessage::Inputs {
            device: device.clone(),
            inputs: LiveInputs {
                pointer: DeviceInput::POINTER,
            },
        })
        .await;
    let activity = open.activity.subscribe();
    let current = *activity.borrow();
    writer
        .control(&LiveModuleMessage::Activity {
            device,
            activity: current,
        })
        .await;
    let screen = open.device.screen();
    let reframes = open.device.frame_changes();
    let mut awake = open.device.awake();
    awake.borrow_and_update();
    let current = Watched {
        source: SourceId::try_from(open.handle.as_str()).expect("a handle fits a picture's source"),
        _watching: open.watched(),
        open,
        handle,
        fit,
        stream,
        activity,
        screen,
        awake,
        area: None,
        framed: None,
        reframes: Some(reframes),
    };
    screen_state(&current, writer).await;
    *watched = Some(current);
}

/// Sends the watched device's frame for the size the view asked, when the
/// view does not have it at that size and orientation yet.
async fn frame(watched: &mut Watched, writer: &Writer<LiveModuleMessage>) {
    let Some(area) = watched.area else { return };
    let screen = *watched.open.device.screen().borrow();
    let orientation = screen.orientation;
    if watched.framed == Some((area, orientation)) {
        return;
    }
    let frame = match watched.open.device.frame(area).await {
        Ok(Some(frame)) => Some(crate::frame::live(frame, watched.open.platform, &watched.open.model, orientation)),
        // The screen's size goes to the view at once, which shows the
        // picture without a frame until the device announces it.
        Ok(None) => None,
        Err(error) => {
            // The drawn frame stands in for a frame the driver could not draw.
            tracing::warn!("{}'s frame was not drawn: {error}", watched.open.name);
            Some(crate::frame::drawn(LiveCorners::round(0.0), watched.open.platform, &watched.open.model, orientation))
        }
    };
    watched.framed = Some((area, orientation));
    writer
        .control(&LiveModuleMessage::Frame {
            device: watched.open.handle.clone(),
            width: area.width,
            height: area.height,
            scale: area.scale,
            orientation,
            points_width: screen.points.x,
            points_height: screen.points.y,
            frame,
        })
        .await;
}

async fn picture(
    watched: &mut Option<Watched>,
    event: Event<Layout>,
    delivery: &mut Delivery,
    writer: &Writer<LiveModuleMessage>,
) {
    let Some(watched) = watched else { return };
    let device = watched.open.handle.clone();
    match event {
        Event::Restart {
            epoch,
            width,
            height,
            layout,
            scale,
            sequence,
        } => {
            let generation = delivery.restart(epoch, width, height, scale, sequence, Some(&watched.stream));
            // The share of the screen's pixels the pictures have, the
            // panel's fit and the path's scale together.
            let pixels = watched.open.device.screen().borrow().pixels;
            let share = fit((width, height), pixels);
            writer
                .control(&LiveModuleMessage::Stream {
                    device,
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
        Event::Frame(frame) => delivery.frame(&watched.source, &frame, Some(&watched.stream)),
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

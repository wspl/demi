//! The simulators' live view on the Host (`mobile.md` § The live view): one
//! invocation of `mobile.live` per view of a conversation's page. A device
//! is a source of the shared live view: viewers of one device share its
//! capture, its pictures are the screen turned upright, and the viewer's
//! input reaches it as touches and keys in the order it arrives, beside the
//! agent's, whose activity the view shows.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use tokio::{
    sync::{mpsc, watch},
    time::MissedTickBehavior,
};
use tokio_util::{
    sync::CancellationToken,
    task::{AbortOnDropHandle, TaskTracker},
};

use demi_command_package_mobile_protocol::{
    live::{INPUT_FAILED, LiveDevice, LiveModuleMessage, LiveViewerMessage},
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
    CONTROL_FRAME, KeyAction, LiveAck, LiveInputs, LiveKeyframe, LiveRelease, PointerAction,
    PointerKind, SourceId, TouchAction, ViewerInput,
};

use crate::{
    devices::{Devices, OpenDevice, Watching},
    driver::{DeviceCapture, FrameArea, KeyPress, Points, TouchPhase},
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
    streams: Mutex<HashMap<String, Arc<StreamHandle<Layout>>>>,
    viewers: AtomicU64,
    tasks: TaskTracker,
}

impl Hub {
    /// Viewer `viewer`'s view of `open`'s pictures.
    fn join(&self, open: &Arc<OpenDevice>, viewer: u64) -> (Arc<StreamHandle<Layout>>, StreamView<Layout>) {
        let mut streams = self.streams.lock().expect("the streams are intact");
        let handle = streams
            .entry(open.native_id.clone())
            .or_insert_with(|| {
                let handle = StreamHandle::start(DeviceSource { open: open.clone() }, &self.tasks);
                // A simulator's screen needs no sizing for its viewers.
                handle.ready();
                Arc::new(handle)
            })
            .clone();
        let view = handle.join(viewer);
        (handle, view)
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
}

struct DeviceCaptureOf(Box<dyn DeviceCapture>);

impl Capture for DeviceCaptureOf {
    async fn next(&mut self) -> Option<CaptureEvent> {
        self.0.next().await
    }

    // The helper encodes as the screen draws and the viewers' writers drop
    // what their paths cannot take, so it needs no window.
    fn ack(&self, _sequence: u32, _window: u32) {}

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
        (
            Layout {
                orientation: screen.orientation,
                points: screen.points,
            },
            (even(width, scale), even(height, scale)),
        )
    }

    fn capturable(&mut self) -> bool {
        true
    }

    fn changed(&mut self, _stopped: bool) -> impl std::future::Future<Output = Change> + Send {
        let mut screen = self.open.device.screen();
        let closed = self.open.closed.clone();
        async move {
            screen.borrow_and_update();
            tokio::select! {
                _ = closed.cancelled() => Change::Gone,
                changed = screen.changed() => match changed {
                    Ok(()) => Change::Layout,
                    Err(_) => Change::Gone,
                },
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
}

/// Delivers one viewer's input to the device it watches.
struct DeviceInput {
    notices: Writer<LiveModuleMessage>,
}

impl DeviceInput {
    async fn failed(&self, error: crate::driver::DriverError) {
        self.notices.notice(INPUT_FAILED, &error.to_string()).await;
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
        let key = match &message {
            ViewerInput::Key { action, code, .. } => keys::usage(code).map(|usage| KeyPress {
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
        let text = match message {
            ViewerInput::Text { text } => Some(text),
            _ => None,
        };
        async move {
            let delivered = if let Some((phase, at)) = step {
                device.touch(phase, at).await
            } else if let Some(press) = key {
                device.key(press).await
            } else if let Some(text) = text {
                device.text(&text).await
            } else {
                Ok(())
            };
            if let Err(error) = delivered {
                self.failed(error).await;
            }
        }
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

/// The finger's step a touch or a mouse's press, drag and release makes,
/// noting where it is in `held`: the first contact is the finger, as a
/// simulator takes one.
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
    stream: StreamView<Layout>,
    activity: watch::Receiver<demi_live_view_protocol::LiveActivity>,
    /// The device's screen, whose turns turn its frame.
    screen: watch::Receiver<crate::driver::Screen>,
    /// The size the view lays the device out at, which its frame fits.
    area: Option<FrameArea>,
    /// The size and the orientation of the frame the view has.
    framed: Option<(FrameArea, Orientation)>,
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
fn read(mut reader: Reader) -> (mpsc::Receiver<Result<LiveViewerMessage, String>>, AbortOnDropHandle<()>) {
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

/// The page's next message; none once it ended its side, a refusal for a
/// frame the protocol refuses.
async fn next(reader: &mut Reader) -> Result<Option<LiveViewerMessage>, String> {
    let Some(frame) = reader.next().await.map_err(|error| error.to_string())? else {
        return Ok(None);
    };
    if frame.kind != CONTROL_FRAME {
        return Err(format!("the page sent a frame of kind {}, which it does not send", frame.kind));
    }
    demi_shared_types::decode_slice(&frame.payload)
        .map(Some)
        .map_err(|error| error.to_string())
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
            Some(LiveViewerMessage::Hello(_)) => {}
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
    let mut ticks = tokio::time::interval(Duration::from_secs(1));
    ticks.set_missed_tick_behavior(MissedTickBehavior::Skip);
    state(devices, conversation, &watched, writer).await;
    let result = loop {
        // The watched device's pictures and the agent's activity on it.
        let from_device = async {
            match watched.as_mut() {
                Some(watched) => tokio::select! {
                    event = watched.stream.events.recv() => FromDevice::Event(event),
                    changed = watched.activity.changed() => FromDevice::Activity(changed.is_ok()),
                    changed = watched.screen.changed() => FromDevice::Turned(changed.is_ok()),
                },
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            _ = cancellation.cancelled() => break Err(None),
            message = inbound.recv() => match message.transpose() {
                Err(error) => break Err(Some(error)),
                Ok(None) => break Ok(()),
                Ok(Some(message)) => match message {
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
                    LiveViewerMessage::Ack(LiveAck { generation, sequence, decode_queue }) => {
                        delivery.acknowledged(generation, sequence, decode_queue, watched.as_ref().map(|watched| &watched.stream));
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
            _ = ticks.tick() => delivery.tick(writer.queued(), watched.as_ref().map(|watched| &watched.stream)),
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
            recording: open.recording.lock().expect("the recording is intact").is_some(),
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
    let (handle, stream) = hub.join(&open, viewer);
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
    *watched = Some(Watched {
        source: SourceId::try_from(open.handle.as_str()).expect("a handle fits a picture's source"),
        _watching: open.watched(),
        open,
        handle,
        stream,
        activity,
        screen,
        area: None,
        framed: None,
    });
}

/// Sends the watched device's frame for the size the view asked, when the
/// view does not have it at that size and orientation yet.
async fn frame(watched: &mut Watched, writer: &Writer<LiveModuleMessage>) {
    let Some(area) = watched.area else { return };
    let orientation = watched.open.device.screen().borrow().orientation;
    if watched.framed == Some((area, orientation)) {
        return;
    }
    let frame = match watched.open.device.frame(area).await {
        Ok(frame) => crate::frame::live(frame, &watched.open.model, orientation),
        Err(error) => {
            // The outline stands in for a frame the driver could not draw.
            tracing::warn!("{}'s frame was not drawn: {error}", watched.open.name);
            crate::frame::outline(0.0, &watched.open.model, orientation)
        }
    };
    watched.framed = Some((area, orientation));
    writer
        .control(&LiveModuleMessage::Frame {
            device: watched.open.handle.clone(),
            orientation,
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
        } => {
            let generation = delivery.restart(epoch, width, height);
            writer
                .control(&LiveModuleMessage::Stream {
                    device,
                    generation,
                    width,
                    height,
                    points_width: layout.points.x,
                    points_height: layout.points.y,
                    orientation: layout.orientation,
                    scale,
                })
                .await;
        }
        Event::Frame(frame) => delivery.frame(&watched.source, &frame, writer, Some(&watched.stream)),
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

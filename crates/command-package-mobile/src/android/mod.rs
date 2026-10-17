//! The Android driver (`mobile.md` § Android): the Host's AVDs, booted with
//! the SDK's emulator without a window; each open emulator's pictures,
//! touches and keys over its own gRPC interface, with no adb in that path;
//! its UI tree and a field's text through `u2.jar` inside it, over adb.
//!
//! Three pixel spaces meet here. The emulator's panel is the screen as the
//! device is built, portrait. Its pictures show the panel turned as the
//! device is held, which its sensors say (the picture's rotation). Android
//! lays its windows out turned as its display is, which follows the
//! sensors only for an app that allows it (the display's rotation): a
//! portrait-locked launcher on a device held sideways is drawn sideways in
//! the picture. Touches and the UI tree are in the display's pixels; the
//! commands' points are the picture's, upright, divided by the screen's
//! density.

mod adb;
pub mod capture;
pub mod emulator;
pub mod guest;
pub mod openh264;
pub mod process;
pub mod sdk;

pub mod proto {
    // The generated server names its stream types after the methods.
    #![allow(non_camel_case_types)]
    tonic::include_proto!("android.emulation.control");
}

use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use futures_util::future::BoxFuture;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use demi_command_package_mobile_protocol::mobile::{Appearance, MobilePlatform, Orientation, PowerState};

use crate::{
    driver::{
        Booting, Button, Device, DeviceCapture, Driver, DriverError, Frame, FrameArea, KeyPress, Png, Points,
        RawElement, Rect, Recording, Result, Screen, Simulator, TouchPhase,
    },
    keys,
};

pub use adb::Adb;
pub use emulator::Discovery;
pub use guest::Guest;
pub use openh264::{Codec, Load};
pub use process::Process;
use proto::{
    ImageFormat, KeyboardEvent, ParameterValue, PhysicalModelValue, Touch, TouchEvent, VmRunState,
    image_format::ImgFormat, keyboard_event, notification, physical_model_value, rotation::SkinRotation,
    vm_run_state::RunState,
};

/// How long an emulator takes from its start to a booted guest.
const BOOTING: Duration = Duration::from_secs(300);
/// How long an emulator takes to end once asked to shut down.
const SHUTTING_DOWN: Duration = Duration::from_secs(60);
/// The density at which a pixel is a point, Android's `mdpi`.
const BASE_DENSITY: f64 = 160.0;
/// The corner radius of an emulator's screen, in points; the emulator
/// does not say it, and a phone's is about this.
const CORNER_RADIUS: f64 = 24.0;
/// The finger a touch moves.
const FINGER: i32 = 1;
/// How long an app may take to show a paste.
const PASTING: Duration = Duration::from_secs(2);
/// The usages of Left Control and of V, which paste.
const CONTROL: u32 = 0xe0;
const V: u32 = 0x19;

/// An AVD, as its files describe it.
#[derive(Debug, Clone, PartialEq)]
pub struct Avd {
    /// The AVD's name, which `emulator -avd` takes.
    pub name: String,
    /// The name Android Studio shows.
    pub display: String,
    pub model: String,
    pub os: String,
}

/// A running emulator and when its process started, which marks its boot.
#[derive(Debug, Clone)]
pub struct Running {
    pub discovery: Discovery,
    pub started: u64,
}

impl Running {
    /// The emulator's process.
    pub fn process(&self) -> Process {
        Process {
            pid: self.discovery.pid,
            started: self.started,
        }
    }
}

/// What the driver needs of the Host's SDK; a test gives its own.
pub trait Sdk: Send + Sync + 'static {
    /// Why this Host cannot run emulators, or none.
    fn unavailable(&self) -> Option<String>;
    fn avds(&self) -> BoxFuture<'_, Result<Vec<Avd>>>;
    fn running(&self) -> BoxFuture<'_, Result<Vec<Running>>>;
    /// Each emulator process with the AVD it runs, whether or not it said
    /// where it listens, as one whose boot hangs has not.
    fn processes(&self) -> BoxFuture<'_, Vec<(String, Process)>>;
    /// Starts the AVD's emulator without a window; answers its process as
    /// soon as it runs, whose start marks the boot.
    fn launch<'a>(&'a self, avd: &'a str) -> BoxFuture<'a, Result<Process>>;
    /// Waits until the emulator `process` says where it listens.
    fn advertised<'a>(&'a self, avd: &'a str, process: Process) -> BoxFuture<'a, Result<Running>>;
    /// Ends once the process has ended.
    fn ended(&self, process: Process) -> BoxFuture<'_, ()>;
    /// Ends the process, when it is still that process, and waits until it
    /// ended.
    fn terminate(&self, process: Process) -> BoxFuture<'_, Result<()>>;
    /// What Demi does inside the emulator adb names `serial`.
    fn guest<'a>(&'a self, serial: &'a str) -> BoxFuture<'a, Result<Arc<dyn Guest>>>;
}

/// The Host's Android emulators.
pub struct Android {
    sdk: Arc<dyn Sdk>,
    codec: Arc<dyn Codec>,
}

impl Android {
    /// The SDK of this Host; an emulator Demi starts writes its log into
    /// `directory`, which keeps Cisco's OpenH264 too, or the system's
    /// temporary directory for a program without an installation.
    pub fn new(directory: Option<std::path::PathBuf>) -> Self {
        let codec = directory
            .clone()
            .unwrap_or_else(|| std::env::temp_dir().join("demi-mobile"))
            .join("openh264");
        Self::with_sdk(Arc::new(sdk::Installed::new(directory)), Arc::new(openh264::Cisco::new(codec)))
    }

    pub fn with_sdk(sdk: Arc<dyn Sdk>, codec: Arc<dyn Codec>) -> Self {
        Self { sdk, codec }
    }

    async fn running(&self, avd: &str) -> Result<Option<Running>> {
        Ok(self.sdk.running().await?.into_iter().find(|running| running.discovery.avd == avd))
    }

    /// The emulator process of the AVD, whether or not it said where it
    /// listens.
    async fn process(&self, avd: &str) -> Option<Process> {
        self.sdk.processes().await.into_iter().find(|(each, _)| each == avd).map(|(_, process)| process)
    }

    /// Asks the emulator `running` to shut down and waits until it ended.
    async fn stop(&self, native_id: &str, running: &Running) -> Result<()> {
        let connected = emulator::Emulator::connect(&running.discovery).await?;
        let mut client = connected.client.clone();
        // The notifications end as the emulator does.
        let mut notifications = client
            .stream_notification(())
            .await
            .map_err(|status| DriverError::Failed(format!("{native_id}: {}", status.message())))?
            .into_inner();
        client
            .set_vm_state(VmRunState {
                state: RunState::Shutdown.into(),
            })
            .await
            .map_err(|status| DriverError::Failed(format!("{native_id} did not shut down: {}", status.message())))?;
        // Its interface goes first, then the process, once it saved
        // its state.
        let ended = async {
            while let Ok(Some(_)) = notifications.message().await {}
            self.sdk.ended(running.process()).await;
        };
        tokio::time::timeout(SHUTTING_DOWN, ended)
            .await
            .map_err(|_| DriverError::Failed(format!("{native_id} did not end in {} s", SHUTTING_DOWN.as_secs())))
    }
}

/// Waits until the guest says it finished booting.
async fn booted(emulator: &mut emulator::Client) -> Result<()> {
    let mut notifications = emulator
        .stream_notification(())
        .await
        .map_err(|status| DriverError::Failed(format!("the emulator's notifications: {}", status.message())))?
        .into_inner();
    loop {
        match notifications.message().await {
            Ok(Some(proto::Notification {
                r#type: Some(notification::Type::Booted(_)),
            })) => return Ok(()),
            Ok(Some(_)) => {}
            Ok(None) => return Err(DriverError::Lost("the emulator ended as it booted".into())),
            Err(status) => return Err(DriverError::Lost(format!("the emulator ended as it booted: {}", status.message()))),
        }
    }
}

impl Driver for Android {
    fn platform(&self) -> MobilePlatform {
        MobilePlatform::Android
    }

    fn unavailable(&self) -> Option<String> {
        self.sdk.unavailable()
    }

    fn list(&self) -> BoxFuture<'_, Result<Vec<Simulator>>> {
        Box::pin(async {
            let running = self.sdk.running().await?;
            let processes = self.sdk.processes().await;
            Ok(self
                .sdk
                .avds()
                .await?
                .into_iter()
                .map(|avd| {
                    let up = running.iter().find(|running| running.discovery.avd == avd.name);
                    // An emulator that has not said where it listens yet is
                    // still booting, or hangs there.
                    let starting = processes.iter().find(|(each, _)| *each == avd.name).map(|(_, process)| process);
                    let (state, boot) = match (up, starting) {
                        (Some(running), _) => (PowerState::Booted, Some(running.started)),
                        (None, Some(process)) => (PowerState::Booting, Some(process.started)),
                        (None, None) => (PowerState::ShutDown, None),
                    };
                    Simulator {
                        native_id: avd.name,
                        name: avd.display,
                        model: avd.model,
                        os: avd.os,
                        state,
                        boot,
                    }
                })
                .collect())
        })
    }

    fn boot<'a>(&'a self, native_id: &'a str, booting: &'a dyn Booting) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let process = self.sdk.launch(native_id).await?;
            // Demi's from its first moment, by the process it started.
            booting.began(process.started);
            let waited = async {
                let running = self.sdk.advertised(native_id, process).await?;
                let connected = emulator::Emulator::connect(&running.discovery).await?;
                let mut client = connected.client.clone();
                tokio::time::timeout(BOOTING, booted(&mut client)).await.map_err(|_| {
                    DriverError::Failed(format!("{native_id} did not finish booting in {} s", BOOTING.as_secs()))
                })?
            };
            let Err(error) = waited.await else { return Ok(()) };
            // The emulator this started for nothing ends again: the process
            // it started, whether or not it ever said where it listens.
            match self.sdk.terminate(process).await {
                Ok(()) => booting.undone(),
                Err(failure) => tracing::warn!("{native_id} stayed up after its boot failed: {failure}"),
            }
            Err(error)
        })
    }

    fn shutdown<'a>(&'a self, native_id: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            if let Some(running) = self.running(native_id).await? {
                match self.stop(native_id, &running).await {
                    Ok(()) => return Ok(()),
                    // An emulator that does not take the request is ended
                    // as a process.
                    Err(error) => tracing::warn!("{native_id} did not shut down when asked: {error}"),
                }
            }
            // One that never said where it listens is ended the same way.
            match self.process(native_id).await {
                Some(process) => self.sdk.terminate(process).await,
                None => Ok(()),
            }
        })
    }

    fn attach<'a>(&'a self, _conversation: &'a str, native_id: &'a str) -> BoxFuture<'a, Result<Arc<dyn Device>>> {
        Box::pin(async move {
            let running = self
                .running(native_id)
                .await?
                .ok_or_else(|| DriverError::Lost(format!("{native_id} is not running")))?;
            let connected = emulator::Emulator::connect(&running.discovery).await?;
            let mut client = connected.client.clone();
            let status = client
                .get_status(())
                .await
                .map_err(|status| DriverError::Failed(format!("{native_id}'s status: {}", status.message())))?
                .into_inner();
            let config = |key: &str| status.platform_config.get(key).and_then(|value| value.parse::<f64>().ok());
            let panel = (
                config("hw.lcd.width").unwrap_or(1080.0),
                config("hw.lcd.height").unwrap_or(2400.0),
            );
            let density = config("hw.lcd.density").unwrap_or(BASE_DENSITY) / BASE_DENSITY;
            let serial = running.discovery.serial();
            let guest = self.sdk.guest(&serial).await?;
            let geometry = Geometry { panel, density };
            let ended = CancellationToken::new();
            let device = Arc::new(AndroidDevice {
                _emulator: connected,
                client,
                guest,
                serial,
                codec: self.codec.clone(),
                geometry,
                screen: watch::Sender::new(geometry.screen(0)),
                drawn: watch::Sender::new(0),
                ended: ended.clone(),
                watching: CancellationToken::new(),
                recording: Mutex::default(),
            });
            device.watch().await?;
            Ok(device as Arc<dyn Device>)
        })
    }
}

/// The screen's sizes, which turning moves between the three spaces.
#[derive(Debug, Clone, Copy)]
struct Geometry {
    /// The panel in pixels, portrait.
    panel: (f64, f64),
    /// Pixels per point.
    density: f64,
}

impl Geometry {
    /// The screen held `turns` quarter turns counterclockwise.
    fn screen(&self, turns: u8) -> Screen {
        let (width, height) = self.turned_size(turns);
        Screen {
            orientation: orientation(turns),
            points: Points {
                x: width / self.density,
                y: height / self.density,
            },
            pixels: (width.round() as u32, height.round() as u32),
        }
    }

    fn turned_size(&self, turns: u8) -> (f64, f64) {
        let (width, height) = self.panel;
        if turns % 2 == 1 { (height, width) } else { (width, height) }
    }

    /// A point of the screen turned `turns`, as the panel has it.
    fn to_panel(&self, (x, y): (f64, f64), turns: u8) -> (f64, f64) {
        let (width, height) = self.panel;
        match turns % 4 {
            0 => (x, y),
            1 => (width - y, x),
            2 => (width - x, height - y),
            _ => (y, height - x),
        }
    }

    /// A point of the panel, on the screen turned `turns`.
    fn from_panel(&self, (x, y): (f64, f64), turns: u8) -> (f64, f64) {
        let (width, height) = self.panel;
        match turns % 4 {
            0 => (x, y),
            1 => (y, width - x),
            2 => (width - x, height - y),
            _ => (height - y, x),
        }
    }

    /// A point of the picture turned `from`, on the display turned `to`.
    fn between(&self, point: (f64, f64), from: u8, to: u8) -> (f64, f64) {
        self.from_panel(self.to_panel(point, from), to)
    }

    /// A rectangle of the display turned `display`, in the picture's points
    /// when the picture is turned `picture`.
    fn rect(&self, rect: &Rect, display: u8, picture: u8) -> Rect {
        let (x1, y1) = self.between((rect.x, rect.y), display, picture);
        let (x2, y2) = self.between((rect.x + rect.width, rect.y + rect.height), display, picture);
        Rect {
            x: x1.min(x2) / self.density,
            y: y1.min(y2) / self.density,
            width: (x1 - x2).abs() / self.density,
            height: (y1 - y2).abs() / self.density,
        }
    }
}

fn orientation(turns: u8) -> Orientation {
    match turns % 4 {
        0 => Orientation::Portrait,
        1 => Orientation::LandscapeLeft,
        2 => Orientation::PortraitUpsideDown,
        _ => Orientation::LandscapeRight,
    }
}

fn turns(orientation: Orientation) -> u8 {
    match orientation {
        Orientation::Portrait => 0,
        Orientation::LandscapeLeft => 1,
        Orientation::PortraitUpsideDown => 2,
        Orientation::LandscapeRight => 3,
    }
}

/// The picture's rotation, in quarter turns counterclockwise.
fn picture_turns(format: Option<&ImageFormat>) -> u8 {
    let rotation = format.and_then(|format| format.rotation.as_ref()).map(|rotation| rotation.rotation());
    match rotation {
        Some(SkinRotation::Landscape) => 1,
        Some(SkinRotation::ReversePortrait) => 2,
        Some(SkinRotation::ReverseLandscape) => 3,
        _ => 0,
    }
}

fn failed(what: &str) -> impl Fn(tonic::Status) -> DriverError + '_ {
    move |status| match status.code() {
        tonic::Code::Unavailable => DriverError::Lost(format!("the emulator is gone: {}", status.message())),
        _ => DriverError::Failed(format!("{what}: {}", status.message())),
    }
}

struct AndroidDevice {
    /// The connection, and the key it registered.
    _emulator: emulator::Emulator,
    client: emulator::Client,
    guest: Arc<dyn Guest>,
    /// adb's name of the emulator.
    serial: String,
    codec: Arc<dyn Codec>,
    geometry: Geometry,
    screen: watch::Sender<Screen>,
    drawn: watch::Sender<u64>,
    ended: CancellationToken,
    /// Stops following the pictures as the device is let go.
    watching: CancellationToken,
    /// The recording under way: the file its MP4 goes to, and since when.
    recording: Mutex<Option<(PathBuf, Instant)>>,
}

impl AndroidDevice {
    /// Follows the emulator's pictures, small, as long as the device is
    /// open: each counts as drawn, and its rotation is the screen's. Their
    /// end is the emulator's.
    async fn watch(&self) -> Result<()> {
        let mut client = self.client.clone();
        let mut stream = client
            .stream_screenshot(ImageFormat {
                format: ImgFormat::Rgb888.into(),
                width: 8,
                height: 8,
                ..ImageFormat::default()
            })
            .await
            .map_err(failed("the emulator's screen"))?
            .into_inner();
        let screen = self.screen.clone();
        let drawn = self.drawn.clone();
        let ended = self.ended.clone();
        let watching = self.watching.clone();
        let geometry = self.geometry;
        // The first picture says how the device is held now.
        let first = stream.message().await.map_err(failed("the emulator's screen"))?;
        if let Some(image) = &first {
            screen.send_replace(geometry.screen(picture_turns(image.format.as_ref())));
        }
        tokio::spawn(async move {
            let followed = async {
                while let Ok(Some(image)) = stream.message().await {
                    drawn.send_modify(|count| *count += 1);
                    let turned = geometry.screen(picture_turns(image.format.as_ref()));
                    screen.send_if_modified(|current| std::mem::replace(current, turned) != turned);
                }
            };
            tokio::select! {
                () = watching.cancelled() => {}
                () = followed => ended.cancel(),
            }
        });
        Ok(())
    }

    fn picture_turns(&self) -> u8 {
        turns(self.screen.borrow().orientation)
    }

    /// `at`, a point of the picture, in the panel's own pixels, which the
    /// emulator's touches take whatever the display shows.
    fn panel_point(&self, at: Points) -> (i32, i32) {
        let pixels = (at.x * self.geometry.density, at.y * self.geometry.density);
        let (x, y) = self.geometry.to_panel(pixels, self.picture_turns());
        (x.round() as i32, y.round() as i32)
    }

    async fn send_touch(&self, at: (i32, i32), down: bool) -> Result<()> {
        send_touch(self.client.clone(), at, down).await
    }

    async fn send_key(&self, event: KeyboardEvent) -> Result<()> {
        send_key(self.client.clone(), event).await
    }

    async fn usb_key(&self, usage: u32, kind: keyboard_event::KeyEventType) -> Result<()> {
        send_key(self.client.clone(), usb(usage, kind)).await
    }

    /// Pastes the clipboard, which holds `text`, at the cursor; returns
    /// once the app took it, so the clipboard may change again.
    async fn paste(&self, text: &str) -> Result<()> {
        // Control-V goes the way the typed keys go, so the paste lands
        // between them in order; the helper's own paste would overtake keys
        // still on their way. The app takes it some time after the
        // emulator did, and nothing says when: each picture after it, the
        // focused field's text is read again, until it holds the text once
        // more.
        let before = self.focused_text().await?;
        let mut drawn = self.drawn.subscribe();
        drawn.borrow_and_update();
        let control = self.key_down(CONTROL).await?;
        self.key_down(V).await?.release().await?;
        control.release().await?;
        let taken = async {
            while drawn.changed().await.is_ok() {
                if pasted(before.as_deref(), self.focused_text().await?.as_deref(), text) {
                    break;
                }
            }
            Ok::<(), DriverError>(())
        };
        // A field whose text the tree withholds, as a password's, is
        // waited for no longer than this.
        match tokio::time::timeout(PASTING, taken).await {
            Ok(read) => read,
            Err(_) => Ok(()),
        }
    }

    /// The focused field's text, as the tree says it now.
    async fn focused_text(&self) -> Result<Option<String>> {
        guest::focused_text(&self.guest.hierarchy().await?)
    }

    /// Puts the finger down at `at`; it is lifted as the answer is dropped
    /// unless lifted before. It counts as down from before the touch is
    /// sent, so a command cancelled while the touch is on its way lifts it.
    async fn finger_down(&self, at: (i32, i32)) -> Result<Pressed> {
        let pressed = Pressed {
            client: self.client.clone(),
            held: Some(Held::Finger(at)),
        };
        self.send_touch(at, true).await?;
        Ok(pressed)
    }

    /// Holds the key `usage` down, as [`Self::finger_down`] holds a finger.
    async fn key_down(&self, usage: u32) -> Result<Pressed> {
        let pressed = Pressed {
            client: self.client.clone(),
            held: Some(Held::Key(usage)),
        };
        self.usb_key(usage, keyboard_event::KeyEventType::Keydown).await?;
        Ok(pressed)
    }
}

async fn send_touch(mut client: emulator::Client, at: (i32, i32), down: bool) -> Result<()> {
    client
        .send_touch(TouchEvent {
            touches: vec![Touch {
                x: at.0,
                y: at.1,
                identifier: FINGER,
                pressure: i32::from(down),
            }],
            display: 0,
        })
        .await
        .map(drop)
        .map_err(failed("the touch"))
}

async fn send_key(mut client: emulator::Client, event: KeyboardEvent) -> Result<()> {
    client.send_key(event).await.map(drop).map_err(failed("the key"))
}

/// A hardware keyboard's key `usage`, by its USB HID usage.
fn usb(usage: u32, kind: keyboard_event::KeyEventType) -> KeyboardEvent {
    KeyboardEvent {
        code_type: keyboard_event::KeyCodeType::Usb.into(),
        event_type: kind.into(),
        key_code: (0x07_0000 | usage) as i32,
        ..KeyboardEvent::default()
    }
}

/// Whether the focused field's text went from `before` to `after` by a
/// paste of `text`: it holds the text more often than before. A field
/// that changed otherwise, as by keys that were still on their way, did
/// not take the paste yet.
fn pasted(before: Option<&str>, after: Option<&str>, text: &str) -> bool {
    let count = |value: Option<&str>| value.map_or(0, |value| value.matches(text).count());
    count(after) > count(before)
}

/// The clipboard as it was before a paste, put back whatever becomes of
/// the paste: at its end, on its failure, or, as the command is cancelled,
/// by a task of its own.
struct Restore {
    guest: Arc<dyn Guest>,
    saved: Option<String>,
}

impl Restore {
    async fn now(&mut self) -> Result<()> {
        match self.saved.take() {
            Some(saved) => self.guest.set_clipboard(&saved).await,
            None => Ok(()),
        }
    }
}

impl Drop for Restore {
    fn drop(&mut self) {
        let Some(saved) = self.saved.take() else { return };
        let guest = self.guest.clone();
        tokio::spawn(async move {
            if let Err(error) = guest.set_clipboard(&saved).await {
                tracing::warn!("the clipboard was not put back after a paste: {error}");
            }
        });
    }
}

/// What a gesture or typing holds down.
#[derive(Debug, Clone, Copy)]
enum Held {
    /// The finger, where it is now.
    Finger((i32, i32)),
    Key(u32),
}

/// A finger or a key held down, released as it is dropped unless released
/// before: a gesture or typing that fails halfway or is cancelled leaves
/// nothing pressed.
struct Pressed {
    client: emulator::Client,
    held: Option<Held>,
}

impl Pressed {
    /// The finger, held down, moved to `at`.
    async fn moved(&mut self, at: (i32, i32)) -> Result<()> {
        self.held = Some(Held::Finger(at));
        send_touch(self.client.clone(), at, true).await
    }

    async fn release(mut self) -> Result<()> {
        match self.held.take() {
            Some(held) => release(self.client.clone(), held).await,
            None => Ok(()),
        }
    }
}

impl Drop for Pressed {
    fn drop(&mut self) {
        let Some(held) = self.held.take() else { return };
        let client = self.client.clone();
        tokio::spawn(async move {
            if let Err(error) = release(client, held).await {
                tracing::warn!("{held:?} was not released: {error}");
            }
        });
    }
}

async fn release(client: emulator::Client, held: Held) -> Result<()> {
    match held {
        Held::Finger(at) => send_touch(client, at, false).await,
        Held::Key(usage) => send_key(client, usb(usage, keyboard_event::KeyEventType::Keyup)).await,
    }
}

/// A finger's path from `from` to `to`, in steps as a person's swipe
/// reports them.
const SWIPE_STEPS: u32 = 12;
const SWIPE_STEP: Duration = Duration::from_millis(16);

impl Device for AndroidDevice {
    fn screen(&self) -> watch::Receiver<Screen> {
        self.screen.subscribe()
    }

    fn drawn(&self) -> watch::Receiver<u64> {
        self.drawn.subscribe()
    }

    fn ended(&self) -> CancellationToken {
        self.ended.clone()
    }

    fn screenshot(&self, long_side: u32) -> BoxFuture<'_, Result<Png>> {
        Box::pin(async move {
            let (width, height) = self.screen.borrow().pixels;
            let factor = (f64::from(long_side) / f64::from(width.max(height).max(1))).min(1.0);
            let image = self
                .client
                .clone()
                .get_screenshot(ImageFormat {
                    format: ImgFormat::Png.into(),
                    width: (f64::from(width) * factor).round() as u32,
                    height: (f64::from(height) * factor).round() as u32,
                    ..ImageFormat::default()
                })
                .await
                .map_err(failed("the screenshot"))?
                .into_inner();
            let (width, height) = image.format.as_ref().map_or((0, 0), |format| (format.width, format.height));
            Ok(Png {
                bytes: image.image,
                width,
                height,
            })
        })
    }

    fn tree(&self) -> BoxFuture<'_, Result<Vec<RawElement>>> {
        Box::pin(async move {
            let xml = self.guest.hierarchy().await?;
            let keyboard = self.guest.keyboard().await;
            let (display, mut elements) = guest::tree(&xml, keyboard.as_deref())?;
            let picture = self.picture_turns();
            fn place(elements: &mut [RawElement], geometry: &Geometry, display: u8, picture: u8) {
                for element in elements {
                    element.frame = geometry.rect(&element.frame, display, picture);
                    place(&mut element.children, geometry, display, picture);
                }
            }
            place(&mut elements, &self.geometry, display, picture);
            Ok(elements)
        })
    }

    fn touch(&self, phase: TouchPhase, at: Points) -> BoxFuture<'_, Result<()>> {
        Box::pin(self.send_touch(self.panel_point(at), phase != TouchPhase::Up))
    }

    fn tap(&self, at: Points, seconds: f64) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            let finger = self.finger_down(self.panel_point(at)).await?;
            if seconds > 0.0 {
                tokio::time::sleep(Duration::from_secs_f64(seconds)).await;
            }
            finger.release().await
        })
    }

    fn swipe(&self, from: Points, to: Points) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            let mut finger = self.finger_down(self.panel_point(from)).await?;
            for step in 1..=SWIPE_STEPS {
                tokio::time::sleep(SWIPE_STEP).await;
                let progress = f64::from(step) / f64::from(SWIPE_STEPS);
                let at = Points {
                    x: from.x + (to.x - from.x) * progress,
                    y: from.y + (to.y - from.y) * progress,
                };
                finger.moved(self.panel_point(at)).await?;
            }
            finger.release().await
        })
    }

    fn press(&self, button: Button) -> BoxFuture<'_, Result<()>> {
        let key = match button {
            Button::Home => "GoHome",
            Button::Back => "GoBack",
            Button::Lock => "Power",
            Button::VolumeUp => "AudioVolumeUp",
            Button::VolumeDown => "AudioVolumeDown",
        };
        Box::pin(self.send_key(KeyboardEvent {
            event_type: keyboard_event::KeyEventType::Keypress.into(),
            key: key.into(),
            ..KeyboardEvent::default()
        }))
    }

    fn key(&self, key: KeyPress) -> BoxFuture<'_, Result<()>> {
        let kind = if key.down {
            keyboard_event::KeyEventType::Keydown
        } else {
            keyboard_event::KeyEventType::Keyup
        };
        Box::pin(self.usb_key(key.usage, kind))
    }

    fn type_keys<'a>(&'a self, typed: &'a [keys::Typed]) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            // Each key down, then up: the emulator keeps these in order with
            // Shift's and Control's, which a key press of its own is not.
            for &(usage, shift) in typed {
                let shift = match shift {
                    true => Some(self.key_down(keys::SHIFT).await?),
                    false => None,
                };
                self.key_down(usage).await?.release().await?;
                if let Some(shift) = shift {
                    shift.release().await?;
                }
            }
            Ok(())
        })
    }

    fn insert<'a>(&'a self, text: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            // At the cursor through the clipboard and a paste, the
            // clipboard put back as it was, whatever becomes of the paste.
            let mut restore = Restore {
                guest: self.guest.clone(),
                saved: Some(self.guest.clipboard().await?),
            };
            self.guest.set_clipboard(text).await?;
            self.paste(text).await?;
            restore.now().await
        })
    }

    fn set_value<'a>(&'a self, at: Points, text: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            // The field at `at` takes the focus, then its text.
            self.tap(at, 0.0).await?;
            self.guest.set_text(text).await
        })
    }

    fn orient(&self, orientation: Orientation) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            let angle = match orientation {
                Orientation::Portrait => 0.0,
                Orientation::LandscapeLeft => 90.0,
                Orientation::PortraitUpsideDown => 180.0,
                Orientation::LandscapeRight => -90.0,
            };
            self.client
                .clone()
                .set_physical_model(PhysicalModelValue {
                    target: physical_model_value::PhysicalType::Rotation.into(),
                    value: Some(ParameterValue { data: vec![0.0, 0.0, angle] }),
                    interpolation: physical_model_value::Interpolation::Step.into(),
                })
                .await
                .map_err(failed("the rotation"))?;
            // The emulator's next picture says the same; the commands that
            // follow already map their points by the new screen.
            self.screen.send_replace(self.geometry.screen(turns(orientation)));
            Ok(())
        })
    }

    fn appearance(&self) -> BoxFuture<'_, Result<Appearance>> {
        Box::pin(async move {
            Ok(if self.guest.night().await? { Appearance::Dark } else { Appearance::Light })
        })
    }

    fn set_appearance(&self, appearance: Appearance) -> BoxFuture<'_, Result<()>> {
        Box::pin(self.guest.set_night(appearance == Appearance::Dark))
    }

    fn record_start<'a>(&'a self, file: &'a std::path::Path) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            self.guest.record_start().await?;
            *self.recording.lock().expect("the recording is intact") = Some((file.to_owned(), Instant::now()));
            Ok(())
        })
    }

    fn record_stop(&self) -> BoxFuture<'_, Result<Recording>> {
        Box::pin(async move {
            let (file, since) = self
                .recording
                .lock()
                .expect("the recording is intact")
                .take()
                .ok_or_else(|| DriverError::Failed("no recording runs".into()))?;
            let duration_ms = u64::try_from(since.elapsed().as_millis()).unwrap_or(u64::MAX);
            self.guest.record_stop(&file).await?;
            Ok(Recording { duration_ms })
        })
    }

    fn frame(&self, _area: FrameArea) -> BoxFuture<'_, Result<Frame>> {
        // Android installs no frame: the view draws the outline.
        Box::pin(async {
            Ok(Frame::Undrawn {
                corner_radius: CORNER_RADIUS,
            })
        })
    }

    fn capture(&self, size: (u32, u32), fps: u32, bitrate: u32) -> BoxFuture<'_, Result<Box<dyn DeviceCapture>>> {
        let client = self.client.clone();
        Box::pin(async move {
            // The first view of an emulator fetches the encoder.
            let load = self.codec.library().await?;
            Ok(Box::new(capture::AndroidCapture::start(client, load, size, fps, bitrate)) as Box<dyn DeviceCapture>)
        })
    }

    fn detach(&self) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            self.watching.cancel();
            self.guest.close().await;
            Ok(())
        })
    }

    fn serial(&self) -> Option<String> {
        Some(self.serial.clone())
    }
}

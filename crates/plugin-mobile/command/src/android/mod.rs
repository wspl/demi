//! The Android driver: the Host's AVDs, booted with the SDK's emulator without
//! a window.
//!
//! - Each open emulator's pictures, touches and keys go over its own gRPC
//!   interface, with no adb in that path.
//! - The emulator's keys reach Android through its hardware keyboard. An AVD
//!   made without one (`hw.keyboard=no`) drops them, so its keys go through
//!   Android's own input injection over adb instead; its configuration is
//!   the user's and stays as it is.
//! - Its UI tree and a field's text come through `u2.jar` inside it, over adb.
//! - Apps are installed, launched and their log followed over adb, as
//!   `adb install`, `am start` and `logcat` do; an erase boots the AVD
//!   with its data wiped, as Android Studio's Wipe Data.
//!
//! # Three pixel spaces
//!
//! Three pixel spaces meet here:
//!
//! - The emulator's panel is the screen as the device is built, portrait.
//! - Its pictures show the panel turned as the device is held, which its
//!   sensors say (the picture's rotation).
//! - Android lays its windows out turned as its display is, which follows the
//!   sensors only for an app that allows it (the display's rotation). A
//!   portrait-locked launcher on a device held sideways is drawn sideways in
//!   the picture.
//!
//! Touches and the UI tree are in the display's pixels. The commands' points
//! are the picture's, upright, divided by the screen's density.

mod adb;
pub mod art;
pub mod axml;
pub mod capture;
pub mod corners;
pub mod emulator;
pub mod guest;
mod logcat;
pub mod process;
pub mod record;
pub mod screen;
pub mod sdk;

pub mod proto {
    // The generated server names its stream types after the methods.
    #![allow(non_camel_case_types)]
    tonic::include_proto!("android.emulation.control");
}

use std::{sync::Arc, time::Duration};

use futures_util::{StreamExt as _, future::BoxFuture};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use demi_shared_recording::Sink;

use demi_command_package_mobile_protocol::{
    mobile::{Appearance, FoldDisplay, MobilePlatform, Orientation, PowerState, SCREENSHOT_LONG_SIDE},
};

use crate::{
    driver::{
        Booting, Button, Device, DeviceCapture, Driver, DriverError, Folded, Frame, FrameArea, Framed, InstalledApp, KeyPress,
        Launched, Png, Points, RawElement, Rect, Result, Screen, Simulator, TouchPhase,
    },
    keys,
    log::{AppEntries, Log},
};

pub use adb::Adb;
pub use emulator::Discovery;
pub use guest::Guest;
pub use demi_shared_openh264::{Codec, Load};
pub use process::Process;
use proto::{
    ImageFormat, KeyboardEvent, MouseEvent, ParameterValue, PhysicalModelValue, VmRunState,
    image_format::ImgFormat, keyboard_event, notification, physical_model_value, posture::PostureValue, rotation::SkinRotation,
    vm_run_state::RunState,
};

/// How long an emulator takes from its start to a booted guest.
const BOOTING: Duration = Duration::from_secs(300);
/// How long an emulator takes to end once asked to shut down.
const SHUTTING_DOWN: Duration = Duration::from_secs(60);
/// The density at which a pixel is a point, Android's `mdpi`.
const BASE_DENSITY: f64 = 160.0;
/// How long an app may take to show a paste.
const PASTING: Duration = Duration::from_secs(2);
/// How long the emulator's pictures may take to show a turn; they follow
/// within a few of its frames.
const TURN: Duration = Duration::from_secs(2);
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
    /// The device it emulates, `hw.device.name`, such as `pixel_10`.
    pub device: String,
    pub os: String,
}

impl Avd {
    /// The device's name in words, such as `Pixel 10`.
    pub fn model(&self) -> String {
        self.device
            .split('_')
            .filter(|word| !word.is_empty())
            .map(|word| {
                let mut characters = word.chars();
                match characters.next() {
                    Some(first) => first.to_uppercase().chain(characters).collect::<String>(),
                    None => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
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
    /// Starts the AVD's emulator without a window, its data wiped first
    /// when `wipe`; answers its process as soon as it runs, whose start
    /// marks the boot.
    fn launch<'a>(&'a self, avd: &'a str, wipe: bool) -> BoxFuture<'a, Result<Process>>;
    /// Waits until the emulator `process` says where it listens.
    fn advertised<'a>(&'a self, avd: &'a str, process: Process) -> BoxFuture<'a, Result<Running>>;
    /// Ends once the process has ended.
    fn ended(&self, process: Process) -> BoxFuture<'_, ()>;
    /// Ends the process, when it is still that process, and waits until it
    /// ended.
    fn terminate(&self, process: Process) -> BoxFuture<'_, Result<()>>;
    /// What Demi does inside the emulator adb names `serial`.
    fn guest<'a>(&'a self, serial: &'a str) -> BoxFuture<'a, Result<Arc<dyn Guest>>>;
    /// The folders of Android Studio's device art for the AVD's `device`,
    /// such as `pixel_10`, when an installation on the Host has it.
    fn device_art(&self, device: &str) -> Option<art::Folders>;
}

/// The Host's Android emulators.
pub struct Android {
    sdk: Arc<dyn Sdk>,
    codec: Arc<dyn Codec>,
}

impl Android {
    /// The SDK of this Host; an emulator Demi starts writes its log into
    /// `directory`, which encodes its emulators' pictures with `codec`.
    pub fn new(directory: Option<std::path::PathBuf>, codec: Arc<dyn Codec>) -> Self {
        Self::with_sdk(Arc::new(sdk::Installed::new(directory)), codec)
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

    /// Boots the AVD, its data wiped first when `wipe`, telling `booting`
    /// as the boot begins, and ends an emulator whose boot failed.
    async fn start(&self, native_id: &str, booting: &dyn Booting, wipe: bool) -> Result<()> {
        let process = self.sdk.launch(native_id, wipe).await?;
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
            Err(failure) => tracing::warn!(event = "emulator.boot_cleanup_failed", native_id, %failure, "an emulator stayed up after its boot failed"),
        }
        Err(error)
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
                        model: avd.model(),
                        native_id: avd.name,
                        name: avd.display,
                        os: avd.os,
                        state,
                        boot,
                    }
                })
                .collect())
        })
    }

    fn boot<'a>(&'a self, native_id: &'a str, booting: &'a dyn Booting) -> BoxFuture<'a, Result<()>> {
        Box::pin(self.start(native_id, booting, false))
    }

    fn erase<'a>(&'a self, native_id: &'a str, booting: &'a dyn Booting) -> BoxFuture<'a, Result<()>> {
        Box::pin(self.start(native_id, booting, true))
    }

    fn shutdown<'a>(&'a self, native_id: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            if let Some(running) = self.running(native_id).await? {
                match self.stop(native_id, &running).await {
                    Ok(()) => return Ok(()),
                    // An emulator that does not take the request is ended
                    // as a process.
                    Err(error) => tracing::warn!(event = "emulator.shutdown_failed", native_id, %error, "an emulator did not shut down when asked"),
                }
            }
            // One that never said where it listens is ended the same way.
            match self.process(native_id).await {
                Some(process) => self.sdk.terminate(process).await,
                None => Ok(()),
            }
        })
    }

    fn attach<'a>(&'a self, _conversation: &'a str, native_id: &'a str, log: Arc<Log>) -> BoxFuture<'a, Result<Arc<dyn Device>>> {
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
            let config = |key: &str| hardware(&status, key).ok_or_else(|| {
                DriverError::Failed(format!("{native_id}'s emulator does not say its screen's {key}"))
            });
            let panel = (config("hw.lcd.width")?, config("hw.lcd.height")?);
            let density = config("hw.lcd.density")? / BASE_DENSITY;
            // A foldable shows a region of its panel while closed, as its
            // hinge's sensors say.
            let folded = hardware(&status, "hw.sensor.hinge.fold_to_displayRegion.0.1_at_posture")
                .and_then(|_| Some((hardware(&status, "hw.displayRegion.0.1.width")?, hardware(&status, "hw.displayRegion.0.1.height")?)))
                .filter(|(width, height)| *width > 0.0 && *height > 0.0);
            // The emulator does not say which device it emulates; its
            // AVD's files do.
            let art = self
                .sdk
                .avds()
                .await?
                .into_iter()
                .find(|avd| avd.name == native_id)
                .and_then(|avd| self.sdk.device_art(&avd.device));
            let serial = running.discovery.serial();
            let guest = self.sdk.guest(&serial).await?;
            let hardware_keyboard = guest.hardware_keyboard().await.unwrap_or_else(|error| {
                tracing::info!(event = "android_keyboard.read_failed", %error, "whether the emulator has a hardware keyboard was not read");
                true
            });
            if let Err(error) = guest.skip_keyboard_tutorials().await {
                tracing::info!(event = "android_tutorials.skip_failed", %error, "the keyboard's tutorials stay on");
            }
            let panels = Panels { open: panel, folded, density };
            let ended = CancellationToken::new();
            let first = panels.screen(0, false);
            // The stream is asked for at most the larger panel's pixels.
            let (largest, largest_points) = match folded {
                Some(folded) if folded.0.max(folded.1) > panel.0.max(panel.1) => {
                    let screen = panels.screen(0, true);
                    (screen.pixels, screen.points)
                }
                _ => (first.pixels, first.points),
            };
            let device = Arc::new(AndroidDevice {
                _emulator: connected,
                pictures: screen::Screen::new(
                    client.clone(),
                    largest,
                    (largest_points.x.round() as u32, largest_points.y.round() as u32),
                    demi_shared_h264::hardware(),
                ),
                client,
                guest,
                serial,
                hardware_keyboard,
                injected_modifiers: std::sync::Mutex::default(),
                codec: self.codec.clone(),
                panels,
                screen: watch::Sender::new(first),
                drawn: watch::Sender::new(0),
                awake: watch::Sender::new(true),
                ended: ended.clone(),
                watching: CancellationToken::new(),
                art_folders: art,
                art: Default::default(),
                corners: Arc::default(),
                corners_read: std::sync::Once::new(),
                frame_known: watch::Sender::new(()),
            });
            if let Err(error) = device.watch().await {
                // What the watch started ends with the device it failed to open.
                device.detach().await?;
                return Err(error);
            }
            tokio::spawn(follow_log(device.guest.clone(), log, device.watching.clone()));
            Ok(device as Arc<dyn Device>)
        })
    }
}

/// How long a fold waits for the emulator to show the other display, and
/// Android to take it up, which takes them a fraction of a second.
const FOLD_LIMIT: Duration = Duration::from_secs(5);
/// How often a fold asks whether Android took up the other display, which
/// each answer of `dumpsys input` takes tens of milliseconds to say.
const TOUCHABLE_POLL: Duration = Duration::from_millis(50);

/// How long the log waits to be followed again once logcat ended, as when
/// the emulator restarts its log daemon.
const LOG_AGAIN: std::time::Duration = std::time::Duration::from_secs(1);

/// Follows the emulator's log into `log` until `stop`, from where the log
/// left off: logcat starts again after it ended, at the last entry's time,
/// whose entries the log holds already.
async fn follow_log(guest: Arc<dyn Guest>, log: Arc<Log>, stop: CancellationToken) {
    let mut since = log.resume_at();
    loop {
        let lines = tokio::select! {
            () = stop.cancelled() => return,
            lines = guest.log(since / 1000.0) => lines,
        };
        match lines {
            Ok(mut lines) => loop {
                let line = tokio::select! {
                    () = stop.cancelled() => return,
                    line = lines.next() => line,
                };
                let Some(line) = line else { break };
                if let Some(entry) = logcat::entry(&line)
                    && entry.timestamp > since
                {
                    log.push(entry);
                }
            },
            Err(error) => tracing::info!(event = "android_log.follow_failed", %error, "the emulator's log was not followed"),
        }
        tokio::select! {
            () = stop.cancelled() => return,
            () = tokio::time::sleep(LOG_AGAIN) => {}
        }
        since = log.resume_at();
    }
}

/// The package and version the APK `file` names in its manifest.
fn manifest(file: &std::path::Path) -> Result<InstalledApp> {
    let unread = |error: &dyn std::fmt::Display| DriverError::Failed(format!("{} is not an APK that reads: {error}", file.display()));
    let apk = std::fs::File::open(file).map_err(|error| unread(&error))?;
    let read = guest::read_apk(&apk, &["AndroidManifest.xml".to_owned()])?;
    let bytes = read.into_iter().next().flatten().ok_or_else(|| unread(&"it has no AndroidManifest.xml"))?;
    let events = axml::events(&bytes).map_err(|error| unread(&error))?;
    let attributes = events
        .into_iter()
        .find_map(|event| match event {
            axml::Event::Start { name, attributes } if &*name == "manifest" => Some(attributes),
            _ => None,
        })
        .ok_or_else(|| unread(&"its manifest has no manifest element"))?;
    let text = |wanted: &str| {
        attributes.iter().find(|attribute| &*attribute.name == wanted).and_then(|attribute| match &attribute.value {
            axml::Value::Text(text) => Some(text.to_string()),
            _ => None,
        })
    };
    Ok(InstalledApp {
        app: text("package").ok_or_else(|| unread(&"its manifest names no package"))?,
        version: text("versionName"),
    })
}

/// The number the emulator's hardware configuration gives `key`, from the
/// map a later emulator sends or the list emulators up to at least 36.6
/// send instead, leaving the map empty.
fn hardware(status: &proto::EmulatorStatus, key: &str) -> Option<f64> {
    let listed = || {
        let entries = status.hardware_config.as_ref()?;
        entries.entry.iter().find(|entry| entry.key == key).map(|entry| entry.value.as_str())
    };
    status.platform_config.get(key).map(String::as_str).or_else(listed)?.parse().ok()
}

/// The size the emulator draws a picture of `pixels` at when asked to fit
/// it into a box whose long side is `long_side` and whose short side is the
/// picture's own: each side scaled by the long side's factor and cut down
/// to a whole pixel, as the emulator does. Never enlarged.
fn emulator_fit((width, height): (u32, u32), long_side: u32) -> (u32, u32) {
    let long = width.max(height).max(1);
    if long <= long_side {
        return (width, height);
    }
    // In whole numbers, so the long side is `long_side` exactly.
    let side = |pixels: u32| ((u64::from(pixels) * u64::from(long_side) / u64::from(long)) as u32).max(1);
    (side(width), side(height))
}

/// The screen's sizes, which turning moves between the three spaces.
#[derive(Debug, Clone, Copy)]
struct Geometry {
    /// The panel in pixels, portrait.
    panel: (f64, f64),
    /// Pixels per point.
    density: f64,
}

/// The emulator's panel open and, for a foldable, the region of it the
/// device shows folded, which the panel is while it is.
#[derive(Debug, Clone, Copy)]
struct Panels {
    open: (f64, f64),
    folded: Option<(f64, f64)>,
    /// Pixels per point.
    density: f64,
}

impl Panels {
    fn geometry(self, folded: bool) -> Geometry {
        Geometry {
            panel: if folded { self.folded.unwrap_or(self.open) } else { self.open },
            density: self.density,
        }
    }

    /// The screen held `turns` quarter turns counterclockwise, folded or
    /// not.
    fn screen(self, turns: u8, folded: bool) -> Screen {
        Screen {
            fold: self.folded.map(|_| Folded::Twofold { open: !folded }),
            ..self.geometry(folded).screen(turns)
        }
    }
}

/// Whether a picture of `format` shows a foldable folded: the region it
/// shows of the panel.
fn folded(format: Option<&ImageFormat>) -> bool {
    format
        .and_then(|format| format.folded_display.as_ref())
        .is_some_and(|region| region.width > 0 && region.height > 0)
}

impl Geometry {
    /// The screen held `turns` quarter turns counterclockwise.
    fn screen(&self, turns: u8) -> Screen {
        let (width, height) = self.turned_size(turns);
        let pixels = (width.round() as u32, height.round() as u32);
        Screen {
            orientation: orientation(turns),
            points: Points {
                x: width / self.density,
                y: height / self.density,
            },
            pixels,
            screenshot: emulator_fit(pixels, SCREENSHOT_LONG_SIDE),
            fold: None,
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
    /// Whether the emulator has a hardware keyboard to take its keys; one
    /// without has its keys injected through the guest.
    hardware_keyboard: bool,
    /// The modifiers held down while keys are injected, which an injected
    /// key is pressed together with.
    injected_modifiers: std::sync::Mutex<Vec<&'static str>>,
    codec: Arc<dyn Codec>,
    /// The emulator's pictures, which the live view and a recording share.
    pictures: Arc<screen::Screen>,
    panels: Panels,
    screen: watch::Sender<Screen>,
    drawn: watch::Sender<u64>,
    /// Whether the display is on, as Android announces it.
    awake: watch::Sender<bool>,
    ended: CancellationToken,
    /// Stops following the pictures and the log as the device is let go.
    watching: CancellationToken,
    /// Android Studio's device art for the device, where the Host has it.
    art_folders: Option<art::Folders>,
    /// The art, read the first time it is drawn; none when it does not read.
    /// The art open and folded, each read the first time it is asked for.
    art: [tokio::sync::OnceCell<Option<Arc<art::Art>>>; 2],
    /// The screen's corners, once read in the background (`read_corners`).
    corners: Arc<std::sync::OnceLock<corners::PanelCorners>>,
    /// Starts that read the first time the drawn frame needs it; the
    /// device art needs none.
    corners_read: std::sync::Once,
    /// Announces the corners once they are known.
    frame_known: watch::Sender<()>,
}

impl AndroidDevice {
    /// The device's art open or folded, read the first time it is asked
    /// for; none folded where the art has no closed posture.
    async fn art(&self, folded: bool) -> Option<Arc<art::Art>> {
        let folders = self.art_folders.as_ref()?;
        let folder = if folded { folders.closed.clone()? } else { folders.open.clone() };
        self.art[usize::from(folded)]
            .get_or_init(|| async move {
                let read = tokio::task::spawn_blocking(move || art::Art::load(&folder).map_err(|error| (folder, error))).await;
                match read {
                    Ok(Ok(art)) => Some(Arc::new(art)),
                    Ok(Err((folder, error))) => {
                        tracing::warn!(event = "android_art.read_failed", folder = %folder.display(), %error, "Android Studio's device art does not read");
                        None
                    }
                    Err(error) => {
                        tracing::warn!(event = "android_art.read_failed", %error, "reading Android Studio's device art stopped");
                        None
                    }
                }
            })
            .await
            .clone()
    }

    /// The whole picture, scaled to the size the emulator's own rule gives
    /// it, and encoded here.
    async fn scaled_screenshot(&self) -> Result<Png> {
        let image = self
            .client
            .clone()
            .get_screenshot(ImageFormat {
                format: ImgFormat::Rgb888.into(),
                ..ImageFormat::default()
            })
            .await
            .map_err(failed("the screenshot"))?
            .into_inner();
        let (width, height) = image.format.as_ref().map_or((0, 0), |format| (format.width, format.height));
        let length = image.image.len();
        let picture = image::RgbImage::from_raw(width, height, image.image.into()).ok_or_else(|| {
            DriverError::Failed(format!("the screenshot's {length} bytes are no {width} × {height} picture"))
        })?;
        // Sized by the picture's own sides, which are the screen's unless
        // it turned meanwhile.
        let (width, height) = emulator_fit((width, height), SCREENSHOT_LONG_SIDE);
        tokio::task::spawn_blocking(move || {
            let scaled = image::imageops::resize(&picture, width, height, image::imageops::FilterType::CatmullRom);
            let mut png = std::io::Cursor::new(Vec::new());
            scaled
                .write_to(&mut png, image::ImageFormat::Png)
                .map_err(|error| DriverError::Failed(format!("the screenshot did not encode: {error}")))?;
            Ok(Png {
                bytes: png.into_inner().into(),
                width,
                height,
            })
        })
        .await
        .map_err(|error| DriverError::Failed(format!("the screenshot was not drawn: {error}")))?
    }

    /// Reads the screen's corners in the background, never holding up the
    /// view (`crates/plugin-mobile/command/src/live.rs`), and announces them once known;
    /// the read ends with the device, or falls back after
    /// [`corners::LIMIT`].
    fn read_corners(&self) {
        let guest = self.guest.clone();
        let corners = self.corners.clone();
        let known = self.frame_known.clone();
        let (watching, ended) = (self.watching.clone(), self.ended.clone());
        let density = self.panels.density;
        tokio::spawn(async move {
            tokio::select! {
                _ = watching.cancelled() => {}
                _ = ended.cancelled() => {}
                read = corners::read(&*guest, density, corners::LIMIT) => {
                    // Read once per device, so never set before.
                    let _ = corners.set(read);
                    known.send_replace(());
                }
            }
        });
    }

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
        let panels = self.panels;
        // The first picture says how the device is held and folded now.
        let first = stream.message().await.map_err(failed("the emulator's screen"))?;
        if let Some(image) = &first {
            let format = image.format.as_ref();
            screen.send_replace(panels.screen(picture_turns(format), folded(format)));
        }
        tokio::spawn(async move {
            let followed = async {
                while let Ok(Some(image)) = stream.message().await {
                    drawn.send_modify(|count| *count += 1);
                    let format = image.format.as_ref();
                    let shown = panels.screen(picture_turns(format), folded(format));
                    screen.send_if_modified(|current| std::mem::replace(current, shown) != shown);
                }
            };
            tokio::select! {
                () = watching.cancelled() => {}
                () = followed => ended.cancel(),
            }
        });
        self.follow_power().await
    }

    /// Follows whether the display is on as long as the device is open,
    /// as Android announces it. A log that ends while the device is open,
    /// as when adb lets go, is followed again from the state read anew; one
    /// that cannot be followed again ends the device, as lost, rather than
    /// leave its last state standing as the display's.
    async fn follow_power(&self) -> Result<()> {
        use futures_util::StreamExt as _;
        let mut turns = self.guest.display().await?;
        if let Some(on) = turns.next().await {
            self.awake.send_replace(on);
        }
        let guest = self.guest.clone();
        let awake = self.awake.clone();
        let watching = self.watching.clone();
        let ended = self.ended.clone();
        tokio::spawn(async move {
            let followed = async {
                let tell = |on: bool| awake.send_if_modified(|current| std::mem::replace(current, on) != on);
                // The first log may end before it told a turn; a log
                // followed again that does is not followed over and over.
                let mut again = false;
                loop {
                    let mut turned = false;
                    while let Some(on) = turns.next().await {
                        turned = true;
                        tell(on);
                    }
                    if again && !turned {
                        tracing::warn!(event = "android_power_log.ended_early", "the display's log ends as soon as it is followed");
                        return;
                    }
                    turns = match guest.display().await {
                        Ok(log) => log,
                        Err(error) => {
                            tracing::warn!(event = "android_power_log.follow_failed", %error, "the display's log cannot be followed again");
                            return;
                        }
                    };
                    again = true;
                    // The first of a log followed again is the state now.
                    match turns.next().await {
                        Some(on) => {
                            tell(on);
                        }
                        None => return,
                    }
                }
            };
            // The pictures' end tells the emulator's; dropping the turns
            // stops following them in the emulator.
            tokio::select! {
                () = watching.cancelled() => {}
                () = ended.cancelled() => {}
                () = followed => ended.cancel(),
            }
        });
        Ok(())
    }

    fn picture_turns(&self) -> u8 {
        turns(self.screen.borrow().orientation)
    }

    /// The panel as the device is folded now.
    fn geometry(&self) -> Geometry {
        self.panels.geometry(matches!(self.screen.borrow().fold, Some(Folded::Twofold { open: false })))
    }

    /// `at`, a point of the picture, in the panel's own pixels, which the
    /// emulator's touches take whatever the display shows.
    fn panel_point(&self, at: Points) -> (i32, i32) {
        let geometry = self.geometry();
        let pixels = (at.x * geometry.density, at.y * geometry.density);
        let (x, y) = geometry.to_panel(pixels, self.picture_turns());
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
        // Control-V goes the way the typed keys go, so the paste lands between
        // them in order. The helper's own paste would overtake keys still on
        // their way.
        //
        // The app takes the paste some time after the emulator did, and nothing
        // says when. So after each picture, the focused field's text is read
        // again, until it holds the text once more.
        let before = self.focused_text().await?;
        let mut drawn = self.drawn.subscribe();
        drawn.borrow_and_update();
        if self.hardware_keyboard {
            let control = self.key_down(CONTROL).await?;
            self.key_down(V).await?.release().await?;
            control.release().await?;
        } else {
            let press = [CONTROL, V].map(|usage| keycode(usage).expect("Control and V have Android's codes"));
            self.guest.inject_keys(&[press.to_vec()]).await?;
        }
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

    /// The key `key` through the guest's injection, on an emulator without a
    /// hardware keyboard: a modifier is held until let go, and another key is
    /// pressed as it goes down, together with the modifiers held.
    async fn inject_key(&self, key: KeyPress) -> Result<()> {
        // A key Android has no code for does nothing, as the emulator's
        // own keyboard ignores it.
        let Some(code) = keycode(key.usage) else {
            return Ok(());
        };
        let press = {
            let mut held = self.injected_modifiers.lock().expect("the held modifiers are intact");
            if (0xe0..=0xe7).contains(&key.usage) {
                held.retain(|modifier| *modifier != code);
                if key.down {
                    held.push(code);
                }
                return Ok(());
            }
            if !key.down {
                return Ok(());
            }
            let mut press = held.clone();
            press.push(code);
            press
        };
        self.guest.inject_keys(&[press]).await
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

/// Touches the display shown at `at`, or lifts the finger, as Android
/// Studio's clicks do: through the emulator's mouse, whose primary button is
/// a finger on that display's touchscreen. The emulator's own touches are
/// scaled by the size of the first display when it has no window, so a
/// closed foldable, which shows its second, takes them elsewhere than sent.
async fn send_touch(mut client: emulator::Client, at: (i32, i32), down: bool) -> Result<()> {
    client
        .send_mouse(MouseEvent {
            x: at.0,
            y: at.1,
            buttons: i32::from(down),
            display: 0,
        })
        .await
        .map(drop)
        .map_err(failed("the touch"))
}

async fn send_key(mut client: emulator::Client, event: KeyboardEvent) -> Result<()> {
    client.send_key(event).await.map(drop).map_err(failed("the key"))
}

/// Android's name of the key of USB HID usage `usage`; none for a key
/// Android has none for.
fn keycode(usage: u32) -> Option<&'static str> {
    const LETTERS: [&str; 26] = [
        "KEYCODE_A", "KEYCODE_B", "KEYCODE_C", "KEYCODE_D", "KEYCODE_E", "KEYCODE_F", "KEYCODE_G", "KEYCODE_H",
        "KEYCODE_I", "KEYCODE_J", "KEYCODE_K", "KEYCODE_L", "KEYCODE_M", "KEYCODE_N", "KEYCODE_O", "KEYCODE_P",
        "KEYCODE_Q", "KEYCODE_R", "KEYCODE_S", "KEYCODE_T", "KEYCODE_U", "KEYCODE_V", "KEYCODE_W", "KEYCODE_X",
        "KEYCODE_Y", "KEYCODE_Z",
    ];
    const DIGITS: [&str; 10] = [
        "KEYCODE_1", "KEYCODE_2", "KEYCODE_3", "KEYCODE_4", "KEYCODE_5", "KEYCODE_6", "KEYCODE_7", "KEYCODE_8",
        "KEYCODE_9", "KEYCODE_0",
    ];
    Some(match usage {
        0x04..=0x1d => LETTERS[(usage - 0x04) as usize],
        0x1e..=0x27 => DIGITS[(usage - 0x1e) as usize],
        0x28 => "KEYCODE_ENTER",
        0x29 => "KEYCODE_ESCAPE",
        0x2a => "KEYCODE_DEL",
        0x2b => "KEYCODE_TAB",
        0x2c => "KEYCODE_SPACE",
        0x2d => "KEYCODE_MINUS",
        0x2e => "KEYCODE_EQUALS",
        0x2f => "KEYCODE_LEFT_BRACKET",
        0x30 => "KEYCODE_RIGHT_BRACKET",
        0x31 => "KEYCODE_BACKSLASH",
        0x33 => "KEYCODE_SEMICOLON",
        0x34 => "KEYCODE_APOSTROPHE",
        0x35 => "KEYCODE_GRAVE",
        0x36 => "KEYCODE_COMMA",
        0x37 => "KEYCODE_PERIOD",
        0x38 => "KEYCODE_SLASH",
        0x39 => "KEYCODE_CAPS_LOCK",
        0x4a => "KEYCODE_MOVE_HOME",
        0x4b => "KEYCODE_PAGE_UP",
        0x4c => "KEYCODE_FORWARD_DEL",
        0x4d => "KEYCODE_MOVE_END",
        0x4e => "KEYCODE_PAGE_DOWN",
        0x4f => "KEYCODE_DPAD_RIGHT",
        0x50 => "KEYCODE_DPAD_LEFT",
        0x51 => "KEYCODE_DPAD_DOWN",
        0x52 => "KEYCODE_DPAD_UP",
        0xe0 => "KEYCODE_CTRL_LEFT",
        0xe1 => "KEYCODE_SHIFT_LEFT",
        0xe2 => "KEYCODE_ALT_LEFT",
        0xe3 => "KEYCODE_META_LEFT",
        0xe4 => "KEYCODE_CTRL_RIGHT",
        0xe5 => "KEYCODE_SHIFT_RIGHT",
        0xe6 => "KEYCODE_ALT_RIGHT",
        0xe7 => "KEYCODE_META_RIGHT",
        _ => return None,
    })
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
                tracing::warn!(event = "android_clipboard.restore_failed", %error, "the clipboard was not put back after a paste");
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
                tracing::warn!(event = "android_input.release_failed", held = ?held, %error, "a held input was not released");
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

    fn awake(&self) -> watch::Receiver<bool> {
        self.awake.subscribe()
    }

    fn screenshot(&self) -> BoxFuture<'_, Result<Png>> {
        Box::pin(async move {
            let screen = *self.screen.borrow();
            let (width, height) = screen.screenshot;
            // The box's long side is the screenshot's and its short side the
            // screen's own, so the long side alone sets the emulator's
            // factor and it draws the screenshot's size (`emulator_fit`).
            let asked = match screen.pixels {
                (wide, high) if wide >= high => (width, high),
                (wide, _) => (wide, height),
            };
            let image = self
                .client
                .clone()
                .get_screenshot(ImageFormat {
                    format: ImgFormat::Png.into(),
                    width: asked.0,
                    height: asked.1,
                    ..ImageFormat::default()
                })
                .await
                .map_err(failed("the screenshot"))?
                .into_inner();
            let drawn = image.format.as_ref().map_or((0, 0), |format| (format.width, format.height));
            if drawn == (width, height) {
                return Ok(Png {
                    bytes: image.image,
                    width,
                    height,
                });
            }
            // An emulator that rounds otherwise, or a picture already
            // turned the other way, is drawn whole and scaled here, a few
            // times slower than the emulator's own (about 220 ms more for a
            // 1080 × 2424 screen): the screenshot is never a size the
            // coordinates are not in.
            tracing::info!(
                event = "android_screenshot.rescaled",
                drawn_w = drawn.0,
                drawn_h = drawn.1,
                width,
                height,
                "the emulator drew another size; its whole picture is scaled"
            );
            self.scaled_screenshot().await
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
            place(&mut elements, &self.geometry(), display, picture);
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
            Button::Lock => return self.guest.power(),
            Button::VolumeUp => "AudioVolumeUp",
            Button::VolumeDown => "AudioVolumeDown",
        };
        Box::pin(self.send_key(KeyboardEvent {
            event_type: keyboard_event::KeyEventType::Keypress.into(),
            key: key.into(),
            ..KeyboardEvent::default()
        }))
    }

    fn wake(&self) -> BoxFuture<'_, Result<()>> {
        self.guest.wake()
    }

    fn key(&self, key: KeyPress) -> BoxFuture<'_, Result<()>> {
        if !self.hardware_keyboard {
            return Box::pin(self.inject_key(key));
        }
        let kind = if key.down {
            keyboard_event::KeyEventType::Keydown
        } else {
            keyboard_event::KeyEventType::Keyup
        };
        Box::pin(self.usb_key(key.usage, kind))
    }

    fn type_keys<'a>(&'a self, typed: &'a [keys::Typed]) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            if !self.hardware_keyboard {
                let presses: Vec<Vec<&'static str>> = typed
                    .iter()
                    .filter_map(|&(usage, shift)| {
                        let key = keycode(usage)?;
                        Some(if shift { vec!["KEYCODE_SHIFT_LEFT", key] } else { vec![key] })
                    })
                    .collect();
                return self.guest.inject_keys(&presses).await;
            }
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

    fn fold_to(&self, degrees: u16) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            if self.panels.folded.is_none() {
                return Err(DriverError::Failed("the device does not fold".into()));
            }
            let open = degrees > 0;
            let posture = if open { PostureValue::PostureOpened } else { PostureValue::PostureClosed };
            let started = std::time::Instant::now();
            let mut screen = self.screen.subscribe();
            self.client
                .clone()
                .set_posture(proto::Posture { value: posture.into() })
                .await
                .map_err(failed("the fold"))?;
            // The emulator shows the other display within a picture or two.
            let shown = tokio::time::timeout(
                FOLD_LIMIT,
                screen.wait_for(|screen| screen.fold == Some(Folded::Twofold { open })),
            )
            .await
            .map(|seen| seen.map(drop));
            tracing::info!(
                event = "mobile_fold.ended",
                serial = %self.serial,
                open,
                ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                outcome = if matches!(shown, Ok(Ok(_))) { "shown" } else { "unshown" },
                "a fold ended"
            );
            match shown {
                Ok(Ok(_)) => {
                    // The emulator shows the other display before Android
                    // takes it up: a touch sent at once is dropped, its
                    // touchscreen's display not yet the one shown. The fold
                    // is done once a touchscreen takes touches there. What
                    // the display then shows is the screen's to settle.
                    let pixels = self.panels.geometry(!open).panel;
                    let taken_up = async {
                        while !self.guest.touchable(pixels).await? {
                            tokio::time::sleep(TOUCHABLE_POLL).await;
                        }
                        Ok::<(), DriverError>(())
                    };
                    let left = FOLD_LIMIT.saturating_sub(started.elapsed());
                    let ms = || u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
                    // The fold happened: a check that fails or runs out of
                    // time leaves it done, as the emulator shows it.
                    match tokio::time::timeout(left, taken_up).await {
                        Ok(Ok(())) => tracing::info!(event = "mobile_fold.taken_up", serial = %self.serial, open, ms = ms(), "Android took up the display a fold shows"),
                        Ok(Err(error)) => tracing::warn!(event = "mobile_fold.unchecked", serial = %self.serial, open, %error, "whether Android took up the display a fold shows was not read"),
                        Err(_) => tracing::warn!(event = "mobile_fold.not_taken_up", serial = %self.serial, open, ms = ms(), "Android did not take up the display a fold shows in time"),
                    }
                    Ok(())
                }
                Ok(Err(_)) => Err(DriverError::Lost("the emulator is gone".into())),
                Err(_) => Err(DriverError::Failed(format!(
                    "the emulator did not show its {} display within {} seconds",
                    if open { "inner" } else { "outer" },
                    FOLD_LIMIT.as_secs()
                ))),
            }
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
            let from = self.screen.borrow().orientation;
            let started = std::time::Instant::now();
            self.client
                .clone()
                .set_physical_model(PhysicalModelValue {
                    target: physical_model_value::PhysicalType::Rotation.into(),
                    value: Some(ParameterValue { data: vec![0.0, 0.0, angle] }),
                    interpolation: physical_model_value::Interpolation::Step.into(),
                })
                .await
                .map_err(failed("the rotation"))?;
            // The screen is the emulator's pictures' to say: the turn is
            // done once they show it, and the commands that follow map their
            // points by the turned screen.
            let mut screen = self.screen.subscribe();
            let shown = tokio::time::timeout(TURN, screen.wait_for(|screen| screen.orientation == orientation)).await;
            let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
            if matches!(shown, Ok(Ok(_))) {
                tracing::info!(event = "mobile_rotation.ended", serial = %self.serial, from = %from, to = %orientation, ms, outcome = "turned", "a rotation ended");
            } else {
                tracing::warn!(event = "mobile_rotation.ended", serial = %self.serial, from = %from, to = %orientation, ms, outcome = "timed_out", "a rotation ended: the emulator's pictures did not turn in time");
            }
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

    fn record(&self, sink: Sink) -> BoxFuture<'_, Result<Box<dyn Send>>> {
        Box::pin(async move {
            // The first recording of an emulator fetches the encoder, which a
            // Host without a hardware one needs.
            let load = self.codec.library().await.map_err(|error| DriverError::Failed(error.to_string()))?;
            Ok(Box::new(record::start(&self.pictures, load, sink)) as Box<dyn Send>)
        })
    }

    fn frame(&self, area: FrameArea) -> BoxFuture<'_, Result<Option<Framed>>> {
        Box::pin(async move {
            // Drawn for the screen as it is now, which the frame names.
            let screen = *self.screen.borrow();
            let turns = turns(screen.orientation);
            // Android Studio's frame where the Host has its art; the
            // frame the view draws otherwise, once the corners it follows
            // are known.
            let folded = matches!(screen.fold, Some(Folded::Twofold { open: false }));
            let Some(art) = self.art(folded).await else {
                self.corners_read.call_once(|| self.read_corners());
                return Ok(self.corners.get().map(|corners| Framed {
                    frame: Frame::Drawn {
                        corners: corners.turned(turns, 1.0 / self.panels.density),
                    },
                    screen,
                }));
            };
            let rendered = tokio::task::spawn_blocking(move || art.render(turns, screen.pixels, area))
                .await
                .map_err(|error| DriverError::Failed(format!("the frame was not drawn: {error}")))?
                .map_err(DriverError::Failed)?;
            Ok(Some(Framed {
                frame: Frame::Image {
                    png: rendered.png.into(),
                    width: rendered.width,
                    height: rendered.height,
                    screen: rendered.screen,
                    // The art's mask covers the picture's corners.
                    silhouette: None,
                },
                screen,
            }))
        })
    }

    fn frame_changes(&self) -> watch::Receiver<()> {
        self.frame_known.subscribe()
    }

    fn capture(
        &self,
        size: (u32, u32),
        fps: u32,
        bitrate: u32,
        display: Option<FoldDisplay>,
    ) -> BoxFuture<'_, Result<Box<dyn DeviceCapture>>> {
        Box::pin(async move {
            // An emulator shows its lit display only.
            if display.is_some() {
                return Err(DriverError::Failed("the device does not fold on a hinge".into()));
            }
            // The first view of an emulator fetches the encoder.
            let load = self.codec.library().await.map_err(|error| DriverError::Failed(error.to_string()))?;
            Ok(Box::new(capture::AndroidCapture::start(&self.pictures, load, size, fps, bitrate)) as Box<dyn DeviceCapture>)
        })
    }

    fn install<'a>(&'a self, file: &'a std::path::Path) -> BoxFuture<'a, Result<InstalledApp>> {
        Box::pin(async move {
            if !file.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("apk")) {
                return Err(DriverError::Failed(format!(
                    "an Android emulator installs an .apk, and {} is not one",
                    file.display()
                )));
            }
            let path = file.to_owned();
            let installed = tokio::task::spawn_blocking(move || manifest(&path))
                .await
                .map_err(|error| DriverError::Failed(format!("reading the APK stopped: {error}")))??;
            self.guest.install(file).await?;
            Ok(installed)
        })
    }

    fn launch<'a>(&'a self, app: &'a str, restart: bool) -> BoxFuture<'a, Result<Launched>> {
        Box::pin(async move {
            let started = self.guest.launch(app, restart).await?;
            // What an app prints goes to its log, as System.out.
            // `am start -W` returns once the first screen is drawn.
            Ok(Launched {
                started,
                console: true,
                shown: true,
            })
        })
    }

    fn app_entries<'a>(&'a self, app: &'a str) -> BoxFuture<'a, Result<AppEntries>> {
        Box::pin(async move { Ok(AppEntries::Uid(self.guest.uid(app).await?)) })
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

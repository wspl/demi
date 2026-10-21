//! The iOS driver: the Host's iOS simulators through `demi-ios-sim`, the helper
//! `demi-mobile` carries.
//!
//! A one-shot helper lists, boots and shuts down simulators. Each conversation
//! that opens an iOS device gets a helper of its own, which its devices share
//! and which ends with the last of them.

mod helper;
#[cfg(target_os = "macos")]
mod install;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, Weak},
};

use futures_util::future::BoxFuture;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

use demi_command_package_mobile_protocol::{
    live::{LiveCorner, LiveCornerPoint, LiveCorners},
    mobile::{Appearance, Orientation, MobilePlatform, PowerState, SCREENSHOT_LONG_SIDE},
};
use demi_live_view::stream::CaptureEvent;

use crate::{
    driver::{
        Booting, Button, Device, DeviceCapture, Driver, DriverError, Frame, FrameArea, Framed, KeyPress, PixelRect, Png, Points,
        RawElement, Rect, Recording, Result, Screen, Simulator, TouchPhase,
    },
    keys,
};

pub use helper::{CONTROL, Connection, VIDEO, BLOB, HEADER_BYTES};
use helper::{Event, Helper};

/// How the driver reaches `demi-ios-sim`: one-shot runs for listing,
/// booting and shutting down, a serving connection per conversation, and
/// what the Host says of its simulators' boots. A test gives its own.
pub trait Launch: Send + Sync + 'static {
    /// Why this Host cannot run the helper, or none.
    fn unavailable(&self) -> Option<String>;
    /// Runs `demi-ios-sim <args>` and answers what it printed.
    fn once<'a>(&'a self, args: &'a [&'a str]) -> BoxFuture<'a, Result<Vec<u8>>>;
    /// Starts `demi-ios-sim serve`.
    fn serve(&self) -> BoxFuture<'_, Result<Connection>>;
    /// Each booted simulator's boot mark, by its simctl id.
    fn boots(&self) -> HashMap<String, u64>;
}

/// The iOS simulators of a Mac.
pub struct Ios {
    launch: Arc<dyn Launch>,
    sessions: Mutex<HashMap<String, Weak<Session>>>,
}

impl Ios {
    /// The Mac's simulators, through the helper this build carries.
    #[cfg(target_os = "macos")]
    pub fn new() -> Self {
        Self::with_launch(Arc::new(Installed::default()))
    }

    /// The simulators `launch` reaches.
    pub fn with_launch(launch: Arc<dyn Launch>) -> Self {
        Self {
            launch,
            sessions: Mutex::default(),
        }
    }

    /// Runs the one-shot `demi-ios-sim <args>` and decodes what it prints.
    async fn once<T: serde::de::DeserializeOwned>(&self, args: &[&str]) -> Result<T> {
        let printed = self.launch.once(args).await?;
        serde_json::from_slice(&printed).map_err(|error| {
            DriverError::Failed(format!("the iOS helper printed what Demi cannot read: {error}"))
        })
    }

    /// `conversation`'s helper, started when it has none.
    async fn session(&self, conversation: &str) -> Result<Arc<Session>> {
        if let Some(session) = self
            .sessions
            .lock()
            .expect("the sessions are intact")
            .get(conversation)
            .and_then(Weak::upgrade)
        {
            return Ok(session);
        }
        let connection = self.launch.serve().await?;
        let (events, heard) = mpsc::unbounded_channel();
        let session = Arc::new(Session {
            helper: Helper::start(connection, events),
            devices: Mutex::default(),
        });
        tokio::spawn(follow(Arc::downgrade(&session), heard));
        let mut sessions = self.sessions.lock().expect("the sessions are intact");
        // A conversation whose helper ended is forgotten with it.
        sessions.retain(|_, session| session.strong_count() > 0);
        sessions.insert(conversation.to_owned(), Arc::downgrade(&session));
        Ok(session)
    }
}

#[cfg(target_os = "macos")]
impl Default for Ios {
    fn default() -> Self {
        Self::new()
    }
}

/// Where Xcode puts the framework every simulator runs on.
#[cfg(target_os = "macos")]
const CORE_SIMULATOR: &str = "/Library/Developer/PrivateFrameworks/CoreSimulator.framework";

/// The oldest macOS the helper runs on.
#[cfg(target_os = "macos")]
const MACOS_MINIMUM: u32 = 15;

/// The helper this build carries, unpacked on first use.
#[cfg(target_os = "macos")]
#[derive(Default)]
struct Installed {
    executable: tokio::sync::OnceCell<std::path::PathBuf>,
}

#[cfg(target_os = "macos")]
impl Installed {
    async fn executable(&self) -> Result<&std::path::Path> {
        self.executable
            .get_or_try_init(install::helper)
            .await
            .map(|path| path.as_path())
    }
}

#[cfg(target_os = "macos")]
impl Launch for Installed {
    fn unavailable(&self) -> Option<String> {
        // idb's libraries, which the helper links, are built for macOS 15 on
        // Apple silicon.
        if cfg!(target_arch = "x86_64") {
            return Some("iOS simulators need a Mac with Apple silicon in this version of Demi".into());
        }
        if !install::carried() {
            return Some("iOS simulators need a build of Demi made with Xcode on a Mac".into());
        }
        let major = sysinfo::System::os_version()
            .and_then(|version| version.split('.').next()?.parse::<u32>().ok());
        if major.is_some_and(|major| major < MACOS_MINIMUM) {
            return Some(format!("iOS simulators need macOS {MACOS_MINIMUM} or later"));
        }
        if !std::path::Path::new(CORE_SIMULATOR).exists() {
            return Some("iOS simulators need Xcode: install it from the App Store and open it once".into());
        }
        None
    }

    fn once<'a>(&'a self, args: &'a [&'a str]) -> BoxFuture<'a, Result<Vec<u8>>> {
        Box::pin(async move {
            let executable = self.executable().await?;
            let output = tokio::process::Command::new(executable)
                .args(args)
                .stdin(std::process::Stdio::null())
                .kill_on_drop(true)
                .output()
                .await
                .map_err(|error| DriverError::Failed(format!("the iOS helper did not start: {error}")))?;
            if !output.status.success() {
                let said = String::from_utf8_lossy(&output.stderr);
                return Err(DriverError::Failed(
                    said.lines().last().unwrap_or("the iOS helper failed").to_owned(),
                ));
            }
            Ok(output.stdout)
        })
    }

    fn serve(&self) -> BoxFuture<'_, Result<Connection>> {
        Box::pin(async move { Connection::spawn(self.executable().await?) })
    }

    /// CoreSimulator starts a `launchd_sim` per boot, whose command line
    /// names the device's data directory (`…/Devices/<udid>/data/var/run/
    /// launchd_bootstrap.plist`), and keeps no boot time of its own a
    /// process outside the boot can read: its start, in seconds since the
    /// epoch, is the mark.
    fn boots(&self) -> HashMap<String, u64> {
        let mut system = sysinfo::System::new();
        system.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::All,
            true,
            sysinfo::ProcessRefreshKind::nothing().with_cmd(sysinfo::UpdateKind::Always),
        );
        system
            .processes()
            .values()
            .filter(|process| process.name() == "launchd_sim")
            .filter_map(|process| {
                let udid = process.cmd().iter().find_map(|argument| {
                    let argument = argument.to_str()?;
                    let (_, rest) = argument.split_once("/Devices/")?;
                    let (udid, _) = rest.split_once("/data/")?;
                    Some(udid.to_owned())
                })?;
                Some((udid, process.start_time()))
            })
            .collect()
    }
}

/// A simulator as the helper lists it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Listed {
    udid: String,
    name: String,
    model: String,
    os: String,
    state: String,
}

fn power(state: &str) -> PowerState {
    match state {
        "Booted" => PowerState::Booted,
        "Booting" => PowerState::Booting,
        "Shutting Down" => PowerState::ShuttingDown,
        _ => PowerState::ShutDown,
    }
}

impl Driver for Ios {
    fn platform(&self) -> MobilePlatform {
        MobilePlatform::Ios
    }

    fn unavailable(&self) -> Option<String> {
        self.launch.unavailable()
    }

    fn list(&self) -> BoxFuture<'_, Result<Vec<Simulator>>> {
        Box::pin(async {
            let listed: Vec<Listed> = self.once(&["list"]).await?;
            let boots = self.launch.boots();
            Ok(listed
                .into_iter()
                .map(|simulator| {
                    let state = power(&simulator.state);
                    Simulator {
                        boot: (state == PowerState::Booted)
                            .then(|| boots.get(&simulator.udid).copied())
                            .flatten(),
                        state,
                        native_id: simulator.udid,
                        name: simulator.name,
                        model: simulator.model,
                        os: simulator.os,
                    }
                })
                .collect())
        })
    }

    fn boot<'a>(&'a self, native_id: &'a str, booting: &'a dyn Booting) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            // simctl's boot is one step: the simulator is booted, or not,
            // when it answers.
            self.once::<Value>(&["boot", native_id]).await?;
            match self.launch.boots().get(native_id) {
                Some(boot) => booting.began(*boot),
                None => tracing::warn!("{native_id}'s boot has no mark; it is not kept past this program"),
            }
            Ok(())
        })
    }

    fn shutdown<'a>(&'a self, native_id: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move { self.once::<Value>(&["shutdown", native_id]).await.map(drop) })
    }

    fn attach<'a>(&'a self, conversation: &'a str, native_id: &'a str) -> BoxFuture<'a, Result<Arc<dyn Device>>> {
        Box::pin(async move {
            let session = self.session(conversation).await?;
            let screen: Turned = session.helper.call("attach", json!({ "udid": native_id })).await?;
            let state = Arc::new(DeviceState {
                screen: watch::Sender::new(screen.screen()),
                held: Mutex::new(Orientation::Portrait),
                drawn: watch::Sender::new(0),
                awake: watch::Sender::new(true),
                ended: session.helper.ended().child_token(),
            });
            session
                .devices
                .lock()
                .expect("the devices are intact")
                .insert(native_id.to_owned(), state.clone());
            Ok(Arc::new(IosDevice {
                session,
                udid: native_id.to_owned(),
                state,
            }) as Arc<dyn Device>)
        })
    }
}

/// One conversation's helper and the devices it has attached.
struct Session {
    helper: Arc<Helper>,
    devices: Mutex<HashMap<String, Arc<DeviceState>>>,
}

/// What the helper's events tell of a device.
struct DeviceState {
    screen: watch::Sender<Screen>,
    /// The orientation the device was last turned to, which the app in
    /// front may have refused: portrait once attached.
    held: Mutex<Orientation>,
    drawn: watch::Sender<u64>,
    /// Whether the screen is on, as the input Demi sent says: a simulator
    /// reports nothing of it.
    awake: watch::Sender<bool>,
    ended: CancellationToken,
}

/// Hands the helper's events to the devices they are about, while the
/// session lives.
async fn follow(session: Weak<Session>, mut events: mpsc::UnboundedReceiver<Event>) {
    while let Some(event) = events.recv().await {
        let Some(session) = session.upgrade() else { return };
        let devices = session.devices.lock().expect("the devices are intact");
        match event {
            Event::Drawn { udid } => {
                if let Some(device) = devices.get(&udid) {
                    device.drawn.send_modify(|count| *count += 1);
                }
            }
            Event::Turned { udid, screen } => {
                if let Some(device) = devices.get(&udid) {
                    let screen = screen.screen();
                    device.screen.send_if_modified(|current| std::mem::replace(current, screen) != screen);
                }
            }
            Event::Lost { udid, reason } => {
                tracing::info!("the iOS simulator {udid} went away: {reason}");
                if let Some(device) = devices.get(&udid) {
                    device.ended.cancel();
                }
            }
            // The helper ended the capture itself; its frames stopped.
            Event::Capture { .. } => {}
        }
    }
}

/// The screen as the helper answers it, upright, in the orientation its
/// display shows.
#[derive(Debug, Clone, Deserialize)]
pub struct Turned {
    orientation: HelperOrientation,
    points: Size,
    pixels: PixelSize,
}

/// An orientation as the helper names it.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
enum HelperOrientation {
    Portrait,
    LandscapeLeft,
    PortraitUpsideDown,
    LandscapeRight,
}

impl From<HelperOrientation> for Orientation {
    fn from(orientation: HelperOrientation) -> Self {
        match orientation {
            HelperOrientation::Portrait => Self::Portrait,
            HelperOrientation::LandscapeLeft => Self::LandscapeLeft,
            HelperOrientation::PortraitUpsideDown => Self::PortraitUpsideDown,
            HelperOrientation::LandscapeRight => Self::LandscapeRight,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct Size {
    width: f64,
    height: f64,
}

#[derive(Debug, Clone, Deserialize)]
struct PixelSize {
    width: u32,
    height: u32,
}

impl Turned {
    fn screen(&self) -> Screen {
        Screen {
            orientation: self.orientation.into(),
            points: Points {
                x: self.points.width,
                y: self.points.height,
            },
            pixels: (self.pixels.width, self.pixels.height),
            // The helper draws the screenshot by the same rule.
            screenshot: crate::scale::fitted((self.pixels.width, self.pixels.height), SCREENSHOT_LONG_SIDE),
        }
    }
}

/// An element as the helper reads it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Element {
    role: String,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    value: Option<String>,
    #[serde(default)]
    identifier: Option<String>,
    frame: ElementFrame,
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    selected: Option<bool>,
    #[serde(default)]
    focused: Option<bool>,
    #[serde(default)]
    secure: bool,
    #[serde(default)]
    keyboard_key: bool,
    #[serde(default)]
    children: Vec<Element>,
}

#[derive(Deserialize)]
struct ElementFrame {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl From<Element> for RawElement {
    fn from(element: Element) -> Self {
        let blank = |text: Option<String>| text.filter(|text| !text.is_empty());
        RawElement {
            role: element.role,
            label: blank(element.label),
            value: blank(element.value),
            identifier: blank(element.identifier),
            frame: Rect {
                x: element.frame.x,
                y: element.frame.y,
                width: element.frame.width,
                height: element.frame.height,
            },
            enabled: element.enabled,
            selected: element.selected,
            focused: element.focused,
            secure: element.secure,
            keyboard_key: element.keyboard_key,
            children: element.children.into_iter().map(Into::into).collect(),
        }
    }
}

struct IosDevice {
    session: Arc<Session>,
    udid: String,
    state: Arc<DeviceState>,
}

impl IosDevice {
    async fn call(&self, method: &str, mut params: Value) -> Result<helper::Answer> {
        params["udid"] = Value::String(self.udid.clone());
        self.session.helper.ask(method, params).await
    }

    async fn done(&self, method: &str, params: Value) -> Result<()> {
        self.call(method, params).await.map(drop)
    }

    async fn call_as<T: serde::de::DeserializeOwned>(&self, method: &str, mut params: Value) -> Result<T> {
        params["udid"] = Value::String(self.udid.clone());
        self.session.helper.call(method, params).await
    }

    /// Sends the input `method`, after which the screen is on: input wakes
    /// it, as the side button, a tap or a key wakes an iPhone.
    async fn input(&self, method: &str, params: Value) -> Result<()> {
        self.done(method, params).await?;
        self.state.awake.send_if_modified(|on| !std::mem::replace(on, true));
        Ok(())
    }
}

/// The screen's corners, for the frame the view draws, from the corner the
/// helper traces on the framebuffer mask: the mask's four corners are
/// alike, and a mask that leaves its corner square has none.
fn mask_corners(corner: Vec<LiveCornerPoint>) -> LiveCorners {
    if corner.len() < 2 {
        return LiveCorners::round(0.0);
    }
    let corner = LiveCorner::Curve { points: corner };
    LiveCorners {
        top_left: corner.clone(),
        top_right: corner.clone(),
        bottom_right: corner.clone(),
        bottom_left: corner,
    }
}

fn orientation_name(orientation: Orientation) -> &'static str {
    match orientation {
        Orientation::Portrait => "portrait",
        Orientation::LandscapeLeft => "landscapeLeft",
        Orientation::PortraitUpsideDown => "portraitUpsideDown",
        Orientation::LandscapeRight => "landscapeRight",
    }
}

impl Device for IosDevice {
    fn screen(&self) -> watch::Receiver<Screen> {
        self.state.screen.subscribe()
    }

    fn held(&self) -> Orientation {
        *self.state.held.lock().expect("the held orientation is intact")
    }

    fn drawn(&self) -> watch::Receiver<u64> {
        self.state.drawn.subscribe()
    }

    fn ended(&self) -> CancellationToken {
        self.state.ended.clone()
    }

    fn awake(&self) -> watch::Receiver<bool> {
        self.state.awake.subscribe()
    }

    fn screenshot(&self) -> BoxFuture<'_, Result<Png>> {
        Box::pin(async move {
            let answer = self.call("screenshot", json!({ "longSide": SCREENSHOT_LONG_SIDE })).await?;
            let size: PixelSize = serde_json::from_value(answer.result)
                .map_err(|error| DriverError::Failed(format!("the screenshot's size does not read: {error}")))?;
            let bytes = answer
                .blob
                .ok_or_else(|| DriverError::Failed("the iOS helper sent no screenshot".into()))?;
            Ok(Png {
                bytes,
                width: size.width,
                height: size.height,
            })
        })
    }

    fn tree(&self) -> BoxFuture<'_, Result<Vec<RawElement>>> {
        Box::pin(async move {
            #[derive(Deserialize)]
            struct Tree {
                elements: Vec<Element>,
            }
            let answer = self.call("tree", json!({})).await?;
            let tree: Tree = serde_json::from_value(answer.result)
                .map_err(|error| DriverError::Failed(format!("the screen's tree does not read: {error}")))?;
            Ok(tree.elements.into_iter().map(Into::into).collect())
        })
    }

    fn touch(&self, phase: TouchPhase, at: Points) -> BoxFuture<'_, Result<()>> {
        let phase = match phase {
            TouchPhase::Down => "down",
            TouchPhase::Move => "move",
            TouchPhase::Up => "up",
        };
        Box::pin(self.input("touch", json!({ "phase": phase, "x": at.x, "y": at.y })))
    }

    fn tap(&self, at: Points, seconds: f64) -> BoxFuture<'_, Result<()>> {
        Box::pin(self.input("tap", json!({ "x": at.x, "y": at.y, "seconds": seconds })))
    }

    fn swipe(&self, from: Points, to: Points) -> BoxFuture<'_, Result<()>> {
        Box::pin(self.input("swipe", json!({ "x1": from.x, "y1": from.y, "x2": to.x, "y2": to.y })))
    }

    fn press(&self, button: Button) -> BoxFuture<'_, Result<()>> {
        let button = match button {
            Button::Home => "home",
            Button::Lock => "lock",
            Button::VolumeUp => "volumeUp",
            Button::VolumeDown => "volumeDown",
            Button::Back => {
                return Box::pin(async { Err(DriverError::Failed("an iPhone has no Back button".into())) });
            }
        };
        let lock = button == "lock";
        Box::pin(async move {
            self.done("button", json!({ "button": button })).await?;
            // The side button turns a screen that is on off, and one that
            // is off on; any other button wakes it.
            self.state.awake.send_modify(|on| *on = !lock || !*on);
            Ok(())
        })
    }

    /// The side button, while the screen is off by the Host's own account;
    /// nothing while it is on, which the side button would turn off.
    fn wake(&self) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            if *self.state.awake.borrow() {
                return Ok(());
            }
            self.press(Button::Lock).await
        })
    }

    fn key(&self, key: KeyPress) -> BoxFuture<'_, Result<()>> {
        Box::pin(self.input("key", json!({ "usage": key.usage, "down": key.down })))
    }

    fn type_keys<'a>(&'a self, keys: &'a [keys::Typed]) -> BoxFuture<'a, Result<()>> {
        // The helper sends the keys as one event, each key released in it.
        Box::pin(self.input("type", json!({ "keys": keys })))
    }

    fn insert<'a>(&'a self, text: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            // A simulator takes no text into its pasteboard from outside
            // (`crates/plugin-mobile/command/src/operations.rs`): the focused
            // field's value becomes
            // its value with the text at its end, through accessibility.
            let tree = self.tree().await?;
            let Some(field) = focused_field(&tree) else {
                return Err(DriverError::NoFocusedField(text.to_owned()));
            };
            if field.secure {
                return Err(DriverError::Failed(format!("{text:?} has no keys on a US keyboard, and the focused field is a password field, whose text cannot be read to add it to")));
            }
            let value = field.value.clone().unwrap_or_default();
            self.set_value(field.frame.centre(), &format!("{value}{text}")).await
        })
    }

    fn set_value<'a>(&'a self, at: Points, text: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(self.input("setValue", json!({ "x": at.x, "y": at.y, "text": text })))
    }

    fn orient(&self, orientation: Orientation) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            let answer = self
                .call("orient", json!({ "orientation": orientation_name(orientation) }))
                .await?;
            let turned: Turned = serde_json::from_value(answer.result)
                .map_err(|error| DriverError::Failed(format!("the turned screen does not read: {error}")))?;
            *self.state.held.lock().expect("the held orientation is intact") = orientation;
            // The orientation the screen took, which an app may have refused.
            let screen = turned.screen();
            self.state.screen.send_if_modified(|current| std::mem::replace(current, screen) != screen);
            Ok(())
        })
    }

    fn appearance(&self) -> BoxFuture<'_, Result<Appearance>> {
        Box::pin(async move {
            #[derive(Deserialize)]
            struct Answer {
                appearance: Appearance,
            }
            let answer = self.call("appearance", json!({})).await?;
            let read: Answer = serde_json::from_value(answer.result)
                .map_err(|error| DriverError::Failed(format!("the appearance does not read: {error}")))?;
            Ok(read.appearance)
        })
    }

    fn set_appearance(&self, appearance: Appearance) -> BoxFuture<'_, Result<()>> {
        Box::pin(self.done("setAppearance", json!({ "appearance": appearance })))
    }

    fn record_start<'a>(&'a self, file: &'a std::path::Path) -> BoxFuture<'a, Result<()>> {
        Box::pin(self.done("recordStart", json!({ "path": file })))
    }

    fn record_stop(&self) -> BoxFuture<'_, Result<Recording>> {
        Box::pin(async move {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Stopped {
                duration_ms: u64,
            }
            let stopped: Stopped = self.call_as("recordStop", json!({})).await?;
            Ok(Recording {
                duration_ms: stopped.duration_ms,
            })
        })
    }

    fn frame(&self, area: FrameArea) -> BoxFuture<'_, Result<Option<Framed>>> {
        Box::pin(async move {
            #[derive(Deserialize)]
            #[serde(rename_all_fields = "camelCase", untagged)]
            enum Answered {
                Drawn { r#for: Turned, width: u32, height: u32, screen: Placed, png_bytes: usize },
                Undrawn { r#for: Turned, corner: Vec<LiveCornerPoint> },
            }
            #[derive(Deserialize)]
            struct Placed {
                x: f64,
                y: f64,
                width: f64,
                height: f64,
            }
            let answer = self.call("frame", json!({ "width": area.width, "height": area.height, "scale": area.scale })).await?;
            let answered: Answered = serde_json::from_value(answer.result)
                .map_err(|error| DriverError::Failed(format!("the device's frame does not read: {error}")))?;
            Ok(Some(match answered {
                Answered::Drawn { r#for, width, height, screen, png_bytes } => {
                    // The blob is the frame's PNG, then the silhouette's.
                    let mut png = answer
                        .blob
                        .ok_or_else(|| DriverError::Failed("the iOS helper sent no frame".into()))?;
                    if png_bytes == 0 || png_bytes >= png.len() {
                        return Err(DriverError::Failed("the iOS helper's frame lacks its silhouette".into()));
                    }
                    let silhouette = png.split_off(png_bytes);
                    Framed {
                        frame: Frame::Image {
                            png,
                            width,
                            height,
                            screen: PixelRect {
                                x: screen.x,
                                y: screen.y,
                                width: screen.width,
                                height: screen.height,
                            },
                            silhouette: Some(silhouette),
                        },
                        screen: r#for.screen(),
                    }
                }
                Answered::Undrawn { r#for, corner } => Framed {
                    frame: Frame::Drawn {
                        corners: mask_corners(corner),
                    },
                    screen: r#for.screen(),
                },
            }))
        })
    }

    fn capture(&self, size: (u32, u32), fps: u32, bitrate: u32) -> BoxFuture<'_, Result<Box<dyn DeviceCapture>>> {
        Box::pin(async move {
            let helper = self.session.helper.clone();
            let capture = helper.next_capture();
            let frames = helper.frames(capture);
            let started = self
                .done(
                    "captureStart",
                    json!({ "capture": capture, "width": size.0, "height": size.1, "fps": fps, "bitrate": bitrate }),
                )
                .await;
            if let Err(error) = started {
                helper.forget(capture);
                return Err(error);
            }
            Ok(Box::new(IosCapture {
                helper,
                udid: self.udid.clone(),
                capture,
                frames,
                acks: std::sync::atomic::AtomicU64::new(0),
            }) as Box<dyn DeviceCapture>)
        })
    }

    fn detach(&self) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            let detached = self.done("detach", json!({})).await;
            self.session
                .devices
                .lock()
                .expect("the devices are intact")
                .remove(&self.udid);
            detached
        })
    }
}

/// A running capture on the helper; dropping it stops it there.
struct IosCapture {
    helper: Arc<Helper>,
    /// The simulator it captures, which each of its calls names.
    udid: String,
    capture: u32,
    frames: mpsc::Receiver<CaptureEvent>,
    /// Numbers the window messages in the order they were sent.
    acks: std::sync::atomic::AtomicU64,
}

impl IosCapture {
    /// Asks the helper `method` for this capture without waiting: its
    /// failure is the Host log's, and the capture's frames say the rest.
    fn tell(&self, method: &'static str, params: Value) {
        let helper = self.helper.clone();
        let mut params = params;
        params["capture"] = Value::from(self.capture);
        params["udid"] = Value::String(self.udid.clone());
        tokio::spawn(async move {
            if let Err(error) = helper.ask(method, params).await {
                tracing::info!("the iOS helper's {method}: {error}");
            }
        });
    }
}

impl DeviceCapture for IosCapture {
    fn next(&mut self) -> BoxFuture<'_, Option<CaptureEvent>> {
        Box::pin(self.frames.recv())
    }

    fn ack(&self, sequence: u32, window: u32) {
        // Each message goes on a task of its own, so they can overtake one
        // another: the helper keeps the newest by its order.
        let order = self.acks.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        self.tell("captureAck", json!({ "sequence": sequence, "window": window, "order": order }));
    }

    fn key_frame(&self) {
        self.tell("captureKeyFrame", json!({}));
    }

    fn encoding(&self, bitrate: u32, fps: u32) {
        self.tell("captureEncoding", json!({ "bitrate": bitrate, "fps": fps }));
    }
}

impl Drop for IosCapture {
    fn drop(&mut self) {
        self.helper.forget(self.capture);
        self.tell("captureStop", json!({}));
    }
}

/// The focused element in `elements`, at any depth: the field the
/// keyboard types into.
fn focused_field(elements: &[RawElement]) -> Option<&RawElement> {
    elements.iter().find_map(|element| {
        if element.focused == Some(true) {
            Some(element)
        } else {
            focused_field(&element.children)
        }
    })
}

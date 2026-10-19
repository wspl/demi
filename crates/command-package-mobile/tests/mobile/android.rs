//! The Android driver against a stub emulator (`mobile.md` § Android): the
//! emulator's own gRPC interface, served here on a local port, which takes
//! only calls signed with the key the driver put into its key directory;
//! and a scripted helper inside it. What the commands print and what
//! reaches the emulator, in the emulator panel's pixels.

use std::{
    path::Path,
    pin::Pin,
    sync::{Arc, Mutex},
};

use aws_lc_rs::signature::{ECDSA_P256_SHA256_FIXED, UnparsedPublicKey};
use base64::Engine as _;
use bytes::Bytes;
use futures_util::{Stream, future::BoxFuture};
use serde_json::{Value, json};
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

use demi_command_package_mobile_protocol::{
    live::{LiveFrame, LiveModuleMessage},
    mobile::PowerState,
};
use demi_mobile::{
    android::{
        Avd, Discovery, Guest, Process, Running, Sdk,
        guest::Resolved,
        proto::{
            EmulatorStatus, Entry, EntryList, Image, ImageFormat, KeyboardEvent, Notification, PhysicalModelValue,
            Rotation, TouchEvent, VmRunState, emulator_controller_server::{EmulatorController, EmulatorControllerServer},
            image_format::ImgFormat, keyboard_event::KeyEventType, notification, rotation::SkinRotation,
        },
    },
    driver::{DriverError, Result},
    testing::FakeDriver,
};

use crate::{CHAT, Mobile, live::{Page, screen}};

/// The AVD the stub emulator runs: a Pixel 8, 1080 × 2400 at 420 dpi.
const AVD: &str = "Pixel_8_API_36";

/// What reached the stub emulator and its helper, in order.
#[derive(Default)]
struct Log {
    lines: Mutex<Vec<String>>,
    /// The number of lines, which a test waits on.
    count: watch::Sender<usize>,
}

impl Log {
    fn push(&self, line: String) {
        self.lines.lock().unwrap().push(line);
        self.count.send_modify(|count| *count += 1);
    }

    fn lines(&self) -> Vec<String> {
        self.lines.lock().unwrap().clone()
    }

    /// Waits until `line` was heard.
    async fn until(&self, line: &str) {
        let mut count = self.count.subscribe();
        let heard = async {
            loop {
                if self.lines().iter().any(|each| each == line) {
                    return;
                }
                count.changed().await.unwrap();
            }
        };
        tokio::time::timeout(std::time::Duration::from_secs(10), heard)
            .await
            .unwrap_or_else(|_| panic!("{line:?} never reached the emulator: {:?}", self.lines()));
    }
}

type Heard = Arc<Log>;

/// The emulator's interface as the stub serves it.
struct StubEmulator {
    screen: Arc<Screen>,
    heard: Heard,
    /// Calls the stub answers with an error, by the line it hears for them.
    refused: Mutex<Vec<String>>,
    /// Cancelled as the emulator stops sending its pictures while it runs.
    pictures_lost: CancellationToken,
    /// The public keys the stub took from its key directory, by id.
    keys: Arc<Mutex<Vec<(String, Vec<u8>)>>>,
    /// The picture's rotation, quarter turns counterclockwise.
    rotation: watch::Sender<u8>,
    /// Each new picture the screen draws.
    drawn: watch::Sender<u64>,
    /// Cancelled as the emulator shuts down.
    ended: CancellationToken,
    /// The picture streams the emulator sends, until their reader drops them.
    streams: watch::Sender<usize>,
    /// The panel in pixels, portrait.
    panel: Mutex<(u32, u32)>,
    /// Whether the status lists the hardware configuration, as emulators
    /// up to at least 36.6 do, rather than map it.
    listed: Mutex<bool>,
    /// Whether a PNG the emulator fits comes out a pixel narrower than its
    /// rule says, as an emulator that rounds otherwise would draw it.
    rounds_otherwise: Mutex<bool>,
}

type Streamed<T> = Pin<Box<dyn Stream<Item = std::result::Result<T, tonic::Status>> + Send>>;

impl StubEmulator {
    /// Refuses a call without a token the registered key signed.
    fn signed<T>(&self, request: &tonic::Request<T>) -> std::result::Result<(), tonic::Status> {
        let refused = |why: &str| Err(tonic::Status::unauthenticated(why.to_owned()));
        let Some(token) = request
            .metadata()
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
        else {
            return refused("no token");
        };
        let parts: Vec<&str> = token.split('.').collect();
        let [header, claims, signature] = parts.as_slice() else { return refused("not a JWT") };
        let decode = |part: &str| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(part).unwrap_or_default();
        let header: Value = serde_json::from_slice(&decode(header)).unwrap_or_default();
        let claims: Value = serde_json::from_slice(&decode(claims)).unwrap_or_default();
        if header.get("typ").is_some() || header["alg"] != "ES256" || claims["iss"] != "android-studio" {
            return refused("a header or claim the emulator refuses");
        }
        let keys = self.keys.lock().unwrap();
        let Some((_, point)) = keys.iter().find(|(id, _)| header["kid"] == id.as_str()) else {
            return refused("an unknown key");
        };
        let message = format!("{}.{}", parts[0], parts[1]);
        UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, point)
            .verify(message.as_bytes(), &decode(signature))
            .or_else(|_| refused("a bad signature"))
    }

    /// Hears `line`, refusing it when the test asked.
    fn hear(&self, line: String) -> std::result::Result<(), tonic::Status> {
        let refused = self.refused.lock().unwrap().contains(&line);
        self.heard.push(line);
        if refused {
            return Err(tonic::Status::internal("the emulator refused it"));
        }
        Ok(())
    }

    /// The picture of `panel` turned `rotation`, within the most asked
    /// for, each side scaled by the smaller factor and cut down to a whole
    /// pixel, as the emulator's are (asked 706 × 1568 of a 1080 × 2424
    /// screen it draws 698 × 1568, asked 698 × 1568 it draws 698 × 1566);
    /// the whole panel when no size is asked.
    fn picture(panel: (u32, u32), rotation: u8, width: u32, height: u32) -> Image {
        let (panel_width, panel_height) = if rotation % 2 == 1 { (panel.1, panel.0) } else { panel };
        // The side whose factor is smaller is the box's; the other is cut
        // down in whole numbers.
        let (width, height) = match (width.min(panel_width).max(1), height.min(panel_height).max(1)) {
            _ if (width, height) == (0, 0) => (panel_width, panel_height),
            (wide, high) if u64::from(wide) * u64::from(panel_height) <= u64::from(high) * u64::from(panel_width) => {
                (wide, (u64::from(panel_height) * u64::from(wide) / u64::from(panel_width)) as u32)
            }
            (_, high) => ((u64::from(panel_width) * u64::from(high) / u64::from(panel_height)) as u32, high),
        };
        Image {
            format: Some(ImageFormat {
                rotation: Some(Rotation {
                    rotation: match rotation {
                        1 => SkinRotation::Landscape,
                        2 => SkinRotation::ReversePortrait,
                        3 => SkinRotation::ReverseLandscape,
                        _ => SkinRotation::Portrait,
                    }
                    .into(),
                }),
                width,
                height,
                ..ImageFormat::default()
            }),
            image: Bytes::from(vec![0x80; (width * height * 3) as usize]),
            ..Image::default()
        }
    }
}

/// The stub as its server serves it.
struct Served(Arc<StubEmulator>);

impl std::ops::Deref for Served {
    type Target = StubEmulator;

    fn deref(&self) -> &StubEmulator {
        &self.0
    }
}

#[tonic::async_trait]
impl EmulatorController for Served {
    async fn set_physical_model(&self, request: tonic::Request<PhysicalModelValue>) -> std::result::Result<tonic::Response<()>, tonic::Status> {
        self.signed(&request)?;
        let angle = request.into_inner().value.unwrap_or_default().data.get(2).copied().unwrap_or(0.0);
        self.hear(format!("rotate {angle}"))?;
        self.rotation.send_replace(((angle as i32).rem_euclid(360) / 90) as u8);
        self.drawn.send_modify(|count| *count += 1);
        Ok(tonic::Response::new(()))
    }

    async fn send_key(&self, request: tonic::Request<KeyboardEvent>) -> std::result::Result<tonic::Response<()>, tonic::Status> {
        self.signed(&request)?;
        let key = request.into_inner();
        if key.key.is_empty() {
            let usage = key.key_code & 0xffff;
            self.hear(format!("usb {usage:#x} {}", key.event_type().as_str_name()))?;
            let down = key.event_type() == KeyEventType::Keydown;
            if usage == 0xe0 {
                *self.screen.control.lock().unwrap() = down;
            }
            // Control-V pastes the clipboard into the focused field.
            if usage == 0x19 && down && *self.screen.control.lock().unwrap() {
                let pasted = self.screen.clipboard.lock().unwrap().clone();
                self.screen.email.lock().unwrap().push_str(&pasted);
            }
        } else {
            // Power reaches an Android 16 guest as nothing, as on a real
            // emulator; the guest's power key turns the display.
            self.hear(format!("key {}", key.key))?;
        }
        // The focused field draws what the key typed.
        self.drawn.send_modify(|count| *count += 1);
        Ok(tonic::Response::new(()))
    }

    async fn send_touch(&self, request: tonic::Request<TouchEvent>) -> std::result::Result<tonic::Response<()>, tonic::Status> {
        self.signed(&request)?;
        for touch in request.into_inner().touches {
            self.hear(format!("touch {},{} {}", touch.x, touch.y, if touch.pressure > 0 { "down" } else { "up" }))?;
        }
        Ok(tonic::Response::new(()))
    }

    async fn get_status(&self, request: tonic::Request<()>) -> std::result::Result<tonic::Response<EmulatorStatus>, tonic::Status> {
        self.signed(&request)?;
        let (width, height) = *self.panel.lock().unwrap();
        let config = [("hw.lcd.width", width.to_string()), ("hw.lcd.height", height.to_string()), ("hw.lcd.density", "420".to_owned())];
        let status = match *self.listed.lock().unwrap() {
            true => EmulatorStatus {
                hardware_config: Some(EntryList {
                    entry: config.into_iter().map(|(key, value)| Entry { key: key.into(), value }).collect(),
                }),
                ..EmulatorStatus::default()
            },
            false => EmulatorStatus {
                platform_config: config.into_iter().map(|(key, value)| (key.into(), value)).collect(),
                ..EmulatorStatus::default()
            },
        };
        Ok(tonic::Response::new(EmulatorStatus { booted: true, ..status }))
    }

    async fn get_screenshot(&self, request: tonic::Request<ImageFormat>) -> std::result::Result<tonic::Response<Image>, tonic::Status> {
        self.signed(&request)?;
        let asked = request.into_inner();
        self.hear(format!("screenshot {} {}x{}", asked.format().as_str_name(), asked.width, asked.height))?;
        let mut image = StubEmulator::picture(*self.panel.lock().unwrap(), *self.rotation.borrow(), asked.width, asked.height);
        if asked.format() == ImgFormat::Png {
            let format = image.format.as_mut().unwrap();
            if *self.rounds_otherwise.lock().unwrap() {
                format.width -= 1;
            }
            let mut png = std::io::Cursor::new(Vec::new());
            image::GrayImage::new(format.width, format.height).write_to(&mut png, image::ImageFormat::Png).unwrap();
            image.image = png.into_inner().into();
        }
        Ok(tonic::Response::new(image))
    }

    type streamScreenshotStream = Streamed<Image>;

    async fn stream_screenshot(&self, request: tonic::Request<ImageFormat>) -> std::result::Result<tonic::Response<Streamed<Image>>, tonic::Status> {
        self.signed(&request)?;
        let asked = request.into_inner();
        let panel = *self.panel.lock().unwrap();
        let (sender, pictures) = mpsc::unbounded_channel();
        let mut drawn = self.drawn.subscribe();
        let rotation = self.rotation.subscribe();
        let ended = self.ended.clone();
        let lost = self.pictures_lost.clone();
        let streams = self.streams.clone();
        streams.send_modify(|count| *count += 1);
        tokio::spawn(async move {
            loop {
                drawn.borrow_and_update();
                if sender.send(Ok(StubEmulator::picture(panel, *rotation.borrow(), asked.width, asked.height))).is_err() {
                    break;
                }
                tokio::select! {
                    () = ended.cancelled() => break,
                    () = lost.cancelled() => break,
                    () = sender.closed() => break,
                    changed = drawn.changed() => if changed.is_err() { break },
                }
            }
            streams.send_modify(|count| *count -= 1);
        });
        Ok(tonic::Response::new(Box::pin(futures_util::stream::unfold(pictures, |mut pictures| async move {
            pictures.recv().await.map(|picture| (picture, pictures))
        }))))
    }

    async fn set_vm_state(&self, request: tonic::Request<VmRunState>) -> std::result::Result<tonic::Response<()>, tonic::Status> {
        self.signed(&request)?;
        self.hear(format!("vm {}", request.into_inner().state().as_str_name()))?;
        self.ended.cancel();
        Ok(tonic::Response::new(()))
    }

    type streamNotificationStream = Streamed<Notification>;

    async fn stream_notification(&self, request: tonic::Request<()>) -> std::result::Result<tonic::Response<Streamed<Notification>>, tonic::Status> {
        self.signed(&request)?;
        let booted = Notification {
            r#type: Some(notification::Type::Booted(Default::default())),
        };
        let ended = self.ended.clone();
        // The current state at once, then nothing until the emulator ends.
        let stream = futures_util::stream::once(async move { Ok(booted) })
            .chain(futures_util::stream::once(async move {
                ended.cancelled().await;
                Err(tonic::Status::unavailable("the emulator ended"))
            }));
        Ok(tonic::Response::new(Box::pin(stream)))
    }
}

use futures_util::StreamExt as _;

/// The helper inside the stub emulator: a sign-up form, the software
/// keyboard below it, and what the driver asks of it.
struct StubGuest {
    heard: Heard,
    /// Android's display rotation, which a portrait-locked app keeps.
    display: Mutex<u8>,
    /// Why the rotation cannot be read, as while Appium holds the UI
    /// automation.
    unreadable: Mutex<Option<String>>,
    night: Mutex<bool>,
    screen: Arc<Screen>,
    /// Cancelled as the display's log ends, as when adb lets go; each
    /// follow of the log takes the token that is current then.
    log: Mutex<CancellationToken>,
    /// Why the display's log cannot be followed, when the test says so.
    log_refused: Mutex<Option<String>>,
    /// What the device says of its screen's corners.
    corners: Mutex<StubCorners>,
    /// Holds each read of the corners while true, as a slow device does.
    held: watch::Sender<bool>,
}

/// A device's screen corners as the stub's Android resolves them.
struct StubCorners {
    /// The radius the display reports, none where it reports none.
    radius: Option<&'static str>,
    /// The APK SystemUI's corner drawables resolve to.
    drawables: &'static str,
    /// The APKs the shell may read, with the drawable each holds.
    apks: std::collections::HashMap<&'static str, Vec<u8>>,
}

/// SystemUI's own APK.
const SYSTEM_UI_APK: &str = "/system_ext/priv-app/SystemUIGoogle/SystemUIGoogle.apk";

impl Default for StubCorners {
    /// A Pixel 10's: a radius of 132 pixels and its drawable in SystemUI's
    /// own APK.
    fn default() -> Self {
        Self {
            radius: Some("132.0px"),
            drawables: SYSTEM_UI_APK,
            apks: [(SYSTEM_UI_APK, include_bytes!("fixtures/rounded_corner.pixel_10.xml").to_vec())].into(),
        }
    }
}

/// Says `line` if it is dropped before it is done, as a cancelled read is.
struct Dropped(Option<Heard>, &'static str);

impl Dropped {
    fn done(mut self) {
        self.0 = None;
    }
}

impl Drop for Dropped {
    fn drop(&mut self) {
        if let Some(heard) = &self.0 {
            heard.push(self.1.into());
        }
    }
}

/// What the stub's app shows and holds: the focused Email field's text,
/// which a paste adds the clipboard to, and the clipboard.
#[derive(Default)]
struct Screen {
    /// Whether the display is off, which the guest's power key turns.
    dark: watch::Sender<bool>,
    email: Mutex<String>,
    clipboard: Mutex<String>,
    /// Whether Control is held, as Control-V pastes.
    control: Mutex<bool>,
}

/// The form, in Android's display pixels for a display turned `rotation`.
fn form(rotation: u8, email: &str) -> String {
    format!(
        r#"<?xml version='1.0' encoding='UTF-8' standalone='yes' ?>
<hierarchy rotation="{rotation}">
  <node index="0" text="" resource-id="" class="android.widget.FrameLayout" package="com.example.shop" content-desc="" clickable="false" enabled="true" focused="false" password="false" selected="false" visible-to-user="true" bounds="[0,0][1080,2400]">
    <node index="0" text="" resource-id="" class="android.widget.LinearLayout" package="com.example.shop" content-desc="" clickable="false" enabled="true" bounds="[0,0][1080,2400]">
      <node index="0" text="{email}" hint="Email" resource-id="com.example.shop:id/email" class="android.widget.EditText" package="com.example.shop" content-desc="" clickable="true" enabled="true" focused="true" password="false" bounds="[100,800][980,900]" />
      <node index="1" text="hunter2" hint="Password" resource-id="com.example.shop:id/password" class="android.widget.EditText" package="com.example.shop" content-desc="" clickable="true" enabled="true" password="true" bounds="[100,920][980,1020]" />
      <node index="2" text="Sign Up" resource-id="com.example.shop:id/sign_up" class="android.widget.Button" package="com.example.shop" content-desc="" clickable="true" enabled="true" bounds="[100,1000][500,1100]" />
      <node index="3" text="Hidden" class="android.widget.TextView" package="com.example.shop" visible-to-user="false" bounds="[0,0][10,10]" />
    </node>
  </node>
  <node index="0" text="" class="android.widget.FrameLayout" package="com.google.android.inputmethod.latin" content-desc="" bounds="[0,1600][1080,2400]">
    <node index="0" text="" class="android.widget.FrameLayout" package="com.google.android.inputmethod.latin" content-desc="q" clickable="true" bounds="[0,1700][108,1800]" />
    <node index="1" text="" class="android.widget.FrameLayout" package="com.google.android.inputmethod.latin" content-desc="w" clickable="true" bounds="[108,1700][216,1800]" />
  </node>
</hierarchy>"#
    )
}

impl Guest for StubGuest {
    fn hierarchy(&self) -> BoxFuture<'_, Result<String>> {
        let email = self.screen.email.lock().unwrap().clone();
        let tree = match self.unreadable.lock().unwrap().clone() {
            Some(why) => Err(demi_mobile::driver::DriverError::Failed(why)),
            None => Ok(form(*self.display.lock().unwrap(), &email)),
        };
        Box::pin(async move { tree })
    }

    fn keyboard(&self) -> BoxFuture<'_, Option<String>> {
        Box::pin(async { Some("com.google.android.inputmethod.latin".to_owned()) })
    }

    fn set_text<'a>(&'a self, text: &'a str) -> BoxFuture<'a, Result<()>> {
        self.heard.push(format!("set text {text}"));
        Box::pin(async { Ok(()) })
    }

    fn clipboard(&self) -> BoxFuture<'_, Result<String>> {
        Box::pin(async { Ok(self.screen.clipboard.lock().unwrap().clone()) })
    }

    /// The wake key turns the display on, and leaves it on.
    fn wake(&self) -> BoxFuture<'_, Result<()>> {
        self.heard.push("wake".into());
        self.screen.dark.send_replace(false);
        Box::pin(async { Ok(()) })
    }

    /// The power key turns the display off, and on again.
    fn power(&self) -> BoxFuture<'_, Result<()>> {
        self.heard.push("power".into());
        self.screen.dark.send_modify(|dark| *dark = !*dark);
        Box::pin(async { Ok(()) })
    }

    /// Whether the display is on now, then each turn, until the log ends.
    fn display(&self) -> BoxFuture<'_, Result<futures_util::stream::BoxStream<'static, bool>>> {
        if let Some(why) = self.log_refused.lock().unwrap().clone() {
            return Box::pin(async { Err(demi_mobile::driver::DriverError::Failed(why)) });
        }
        let log = self.log.lock().unwrap().clone();
        let mut dark = self.screen.dark.subscribe();
        let now = !*dark.borrow_and_update();
        let turns = futures_util::stream::unfold(dark, |mut dark| async move {
            dark.changed().await.ok()?;
            let on = !*dark.borrow_and_update();
            Some((on, dark))
        });
        let turns = futures_util::stream::once(std::future::ready(now)).chain(turns).take_until(log.cancelled_owned());
        Box::pin(async move { Ok(turns.boxed()) })
    }

    fn set_clipboard<'a>(&'a self, text: &'a str) -> BoxFuture<'a, Result<()>> {
        self.heard.push(format!("clipboard {text}"));
        *self.screen.clipboard.lock().unwrap() = text.to_owned();
        Box::pin(async { Ok(()) })
    }

    fn night(&self) -> BoxFuture<'_, Result<bool>> {
        Box::pin(async { Ok(*self.night.lock().unwrap()) })
    }

    fn set_night(&self, dark: bool) -> BoxFuture<'_, Result<()>> {
        *self.night.lock().unwrap() = dark;
        self.heard.push(format!("night {dark}"));
        Box::pin(async { Ok(()) })
    }

    fn record_start(&self) -> BoxFuture<'_, Result<()>> {
        Box::pin(async { Ok(()) })
    }

    fn record_stop<'a>(&'a self, file: &'a Path) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            tokio::fs::write(file, b"MP4").await.unwrap();
            Ok(())
        })
    }

    /// The corners the test gives, once the test lets the read go on.
    fn resource<'a>(&'a self, package: &'a str, name: &'a str) -> BoxFuture<'a, Result<Resolved>> {
        Box::pin(async move {
            let reading = Dropped(Some(self.heard.clone()), "corner read let go");
            let mut held = self.held.subscribe();
            // The sender lives with the guest.
            let _ = held.wait_for(|held| !*held).await;
            reading.done();
            let corners = self.corners.lock().unwrap();
            let resolved = |value: &str, apk: &str| Resolved { value: value.into(), apk: Some(apk.into()) };
            match (package, name) {
                ("android", radius) if radius.starts_with("android:dimen/rounded_corner_radius") => corners
                    .radius
                    .map(|radius| resolved(radius, "/data/app/com.android.internal.emulation.pixel_10/base.apk"))
                    .ok_or_else(|| DriverError::Failed(format!("{name} does not resolve"))),
                ("com.android.systemui", drawable) if drawable.starts_with("com.android.systemui:drawable/rounded_corner_") => {
                    let file = drawable.trim_start_matches("com.android.systemui:drawable/");
                    Ok(resolved(&format!("res/drawable/{file}.xml"), corners.drawables))
                }
                _ => Err(DriverError::Failed(format!("{name} does not resolve"))),
            }
        })
    }

    fn apk_files<'a>(&'a self, apk: &'a str, paths: &'a [String]) -> BoxFuture<'a, Result<Vec<Option<Vec<u8>>>>> {
        self.heard.push(format!("apk files {apk} {}", paths.join(" ")));
        let files = match self.corners.lock().unwrap().apks.get(apk) {
            Some(file) => Ok(paths.iter().map(|_| Some(file.clone())).collect()),
            None => Err(DriverError::Failed(format!("{apk}: Permission denied"))),
        };
        Box::pin(async move { files })
    }

    fn close(&self) -> BoxFuture<'_, ()> {
        self.heard.push("guest closed".into());
        Box::pin(async {})
    }
}

/// An SDK with two AVDs, of which the stub emulator boots the first.
struct StubSdk {
    emulator: Arc<StubEmulator>,
    guest: Arc<StubGuest>,
    discovery: Discovery,
    /// Whether the emulator runs and said where it listens.
    up: Mutex<bool>,
    /// Whether the emulator's process runs.
    launched: Mutex<bool>,
    /// Whether a started emulator never says where it listens.
    silent: Mutex<bool>,
    /// Whether ending an emulator's process fails.
    unkillable: Mutex<bool>,
    /// Android Studio's art for the stub's Pixel 8, when the test gives it.
    art: Mutex<Option<std::path::PathBuf>>,
    /// Takes in the driver's keys while the stub lives.
    _keys: notify::RecommendedWatcher,
    _directory: tempfile::TempDir,
    _server: tokio_util::task::AbortOnDropHandle<()>,
}

impl StubSdk {
    async fn start(up: bool) -> Arc<Self> {
        let directory = tempfile::tempdir().unwrap();
        let jwks = directory.path().join("jwks").join("set");
        std::fs::create_dir_all(&jwks).unwrap();
        let active = jwks.join("active.jwk");
        std::fs::write(&active, r#"{"keys": []}"#).unwrap();
        let heard = Heard::default();
        let screen = Arc::new(Screen::default());
        *screen.clipboard.lock().unwrap() = "the user's copy".into();
        let emulator = Arc::new(StubEmulator {
            screen: screen.clone(),
            heard: heard.clone(),
            refused: Mutex::default(),
            pictures_lost: CancellationToken::new(),
            keys: Arc::default(),
            rotation: watch::Sender::new(0),
            drawn: watch::Sender::new(0),
            ended: CancellationToken::new(),
            streams: watch::Sender::new(0),
            panel: Mutex::new((1080, 2400)),
            listed: Mutex::new(false),
            rounds_otherwise: Mutex::new(false),
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let service = EmulatorControllerServer::new(Served(emulator.clone()));
        let server = tokio_util::task::AbortOnDropHandle::new(tokio::spawn(async move {
            // The server ends with the test.
            let _ = tonic::transport::Server::builder()
                .add_service(service)
                .serve_with_incoming(tonic::transport::server::TcpIncoming::from(listener))
                .await;
        }));
        let keys = take_keys(&jwks, &active, emulator.keys.clone());
        Arc::new(Self {
            guest: Arc::new(StubGuest {
                heard,
                display: Mutex::new(0),
                unreadable: Mutex::new(None),
                night: Mutex::new(false),
                screen: screen.clone(),
                log: Mutex::new(CancellationToken::new()),
                log_refused: Mutex::new(None),
                corners: Mutex::default(),
                held: watch::Sender::new(false),
            }),
            discovery: Discovery {
                pid: 4242,
                avd: AVD.into(),
                grpc_port: port,
                console_port: 5554,
                jwks: Some(jwks),
                jwk_active: Some(active),
                token: None,
            },
            emulator,
            launched: Mutex::new(up),
            up: Mutex::new(up),
            silent: Mutex::new(false),
            unkillable: Mutex::new(false),
            art: Mutex::new(None),
            _keys: keys,
            _directory: directory,
            _server: server,
        })
    }

    fn heard(&self) -> Vec<String> {
        self.emulator.heard.lines()
    }

    /// Refuses the call heard as `line`.
    fn refuse(&self, line: &str) {
        self.emulator.refused.lock().unwrap().push(line.to_owned());
    }

    fn running(&self) -> Running {
        Running {
            discovery: self.discovery.clone(),
            started: 1_000,
        }
    }
}

/// Takes in each key file the driver puts into `jwks` and lists its id in
/// `active`, as the emulator does, while the watcher it answers lives.
fn take_keys(jwks: &Path, active: &Path, keys: Arc<Mutex<Vec<(String, Vec<u8>)>>>) -> notify::RecommendedWatcher {
    use notify::Watcher as _;
    let (jwks, active) = (jwks.to_owned(), active.to_owned());
    let directory = jwks.clone();
    let mut watcher = notify::recommended_watcher(move |_event| {
        // The directory goes as the test ends.
        let Ok(entries) = std::fs::read_dir(&jwks) else { return };
        for entry in entries.flatten() {
            if entry.path() == active || entry.path().extension().is_none_or(|extension| extension != "jwk") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(entry.path()) else { continue };
            let Ok(set) = serde_json::from_str::<Value>(&text) else { continue };
            for key in set["keys"].as_array().into_iter().flatten() {
                let decode = |part: &Value| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(part.as_str().unwrap_or_default()).unwrap_or_default();
                let mut point = vec![4];
                point.extend(decode(&key["x"]));
                point.extend(decode(&key["y"]));
                let id = key["kid"].as_str().unwrap_or_default().to_owned();
                let mut keys = keys.lock().unwrap();
                if !keys.iter().any(|(known, _)| *known == id) {
                    keys.push((id, point));
                }
            }
        }
        let ids: Vec<Value> = keys.lock().unwrap().iter().map(|(id, _)| json!({ "kid": id })).collect();
        let listed = json!({ "keys": ids }).to_string();
        // Written only when it changes, as its own write is an event too.
        if std::fs::read_to_string(&active).ok().as_deref() != Some(listed.as_str()) {
            let _ = std::fs::write(&active, listed);
        }
    })
    .unwrap();
    watcher.watch(&directory, notify::RecursiveMode::NonRecursive).unwrap();
    watcher
}

impl Sdk for StubSdk {
    fn unavailable(&self) -> Option<String> {
        None
    }

    fn avds(&self) -> BoxFuture<'_, Result<Vec<Avd>>> {
        Box::pin(async {
            Ok(vec![
                Avd { name: "Medium_Phone_API_35".into(), display: "Medium Phone API 35".into(), device: "medium_phone".into(), os: "Android 15".into() },
                Avd { name: AVD.into(), display: "Pixel 8 API 36".into(), device: "pixel_8".into(), os: "Android 16".into() },
            ])
        })
    }

    fn running(&self) -> BoxFuture<'_, Result<Vec<Running>>> {
        Box::pin(async { Ok(if *self.up.lock().unwrap() && !self.emulator.ended.is_cancelled() { vec![self.running()] } else { Vec::new() }) })
    }

    fn processes(&self) -> BoxFuture<'_, Vec<(String, Process)>> {
        let launched = *self.launched.lock().unwrap() && !self.emulator.ended.is_cancelled();
        Box::pin(async move { if launched { vec![(AVD.to_owned(), self.running().process())] } else { Vec::new() } })
    }

    fn launch<'a>(&'a self, avd: &'a str) -> BoxFuture<'a, Result<Process>> {
        let _heard = self.emulator.hear(format!("launch {avd}"));
        *self.launched.lock().unwrap() = true;
        Box::pin(async { Ok(self.running().process()) })
    }

    fn advertised<'a>(&'a self, avd: &'a str, process: Process) -> BoxFuture<'a, Result<Running>> {
        assert_eq!(process, self.running().process());
        let advertised = match *self.silent.lock().unwrap() {
            true => Err(demi_mobile::driver::DriverError::Failed(format!("{avd} did not start in 120 s"))),
            false => {
                *self.up.lock().unwrap() = true;
                Ok(self.running())
            }
        };
        Box::pin(async { advertised })
    }

    fn ended(&self, _process: Process) -> BoxFuture<'_, ()> {
        Box::pin(self.emulator.ended.cancelled())
    }

    fn terminate(&self, process: Process) -> BoxFuture<'_, Result<()>> {
        let _heard = self.emulator.hear(format!("terminate {} {}", process.pid, process.started));
        let ended = match *self.unkillable.lock().unwrap() {
            true => Err(demi_mobile::driver::DriverError::Failed("the process did not end".into())),
            false => {
                self.emulator.ended.cancel();
                Ok(())
            }
        };
        Box::pin(async { ended })
    }

    fn guest<'a>(&'a self, serial: &'a str) -> BoxFuture<'a, Result<Arc<dyn Guest>>> {
        assert_eq!(serial, "emulator-5554");
        Box::pin(async { Ok(self.guest.clone() as Arc<dyn Guest>) })
    }

    fn device_art(&self, device: &str) -> Option<std::path::PathBuf> {
        assert_eq!(device, "pixel_8");
        self.art.lock().unwrap().clone()
    }
}

async fn start(up: bool) -> (Mobile, Arc<StubSdk>) {
    let sdk = StubSdk::start(up).await;
    let simulators = vec![FakeDriver::simulator("UDID-1", "iPhone 17", PowerState::ShutDown)];
    (Mobile::start_android(simulators, sdk.clone()).await, sdk)
}

/// The ref of the element labelled `label`.
fn reference(elements: &Value, label: &str) -> Option<String> {
    for each in elements.as_array()? {
        if each["label"] == label {
            return each["ref"].as_str().map(str::to_owned);
        }
        if let Some(found) = reference(&each["children"], label) {
            return Some(found);
        }
    }
    None
}

/// `list` shows the AVDs beside the iOS simulators in one table, each by
/// the AVD name `open` takes. Planted defect this catches: Android listed
/// apart, or by its display name, which `open` does not take. A stub SDK,
/// milliseconds.
#[tokio::test]
async fn avds_are_listed_beside_the_ios_simulators() {
    let (mobile, _sdk) = start(false).await;
    let text = mobile.run_text(CHAT, "list", json!({})).await;
    assert_eq!(
        String::from_utf8(text.stdout).unwrap(),
        "ID                   PLATFORM  OS          STATE      HANDLE  MODEL         NAME\n\
         UDID-1               ios       iOS 26.5    shut down  -       iPhone 17     iPhone 17\n\
         Medium_Phone_API_35  android   Android 15  shut down  -       Medium Phone  Medium Phone API 35\n\
         Pixel_8_API_36       android   Android 16  shut down  -       Pixel 8       Pixel 8 API 36\n"
    );
    assert_eq!(text.stderr, "");
}

/// A Host whose SDK has no emulator package says how to install it, as
/// `list`'s warning and as `open`'s failure. Planted defect this catches:
/// the AVDs listed with no way to boot them. An empty SDK directory,
/// milliseconds.
#[tokio::test]
async fn a_host_without_the_emulator_says_how_to_install_it() {
    let sdk = tempfile::tempdir().unwrap();
    let installed = demi_mobile::android::sdk::Installed::at(Some(sdk.path().to_owned()), None);
    let mobile = Mobile::start_android(Vec::new(), Arc::new(installed)).await;
    let text = mobile.run_text(CHAT, "list", json!({})).await;
    assert_eq!(
        text.stderr,
        "demi mobile list: warning: no Android emulator; install the Android SDK's emulator package (sdkmanager \"emulator\")\n"
    );
    let (code, message) = mobile.run(CHAT, "open", json!({ "device": AVD })).await.failure();
    assert_eq!(code, "device_not_found");
    assert!(message.contains("no Android emulator; install the Android SDK's emulator package (sdkmanager \"emulator\")"), "{message}");
}

/// An AVD that is shut down boots, is driven over the emulator's gRPC
/// interface with signed tokens, and shuts down as it closes: taps in
/// panel pixels, keys as a hardware keyboard's, a field's text through
/// the helper, the dark appearance and a quarter turn. Planted defects
/// this catches: a call without the token (the stub refuses it), a touch
/// in screenshot pixels, Back sent to an iPhone's code path. A stub
/// emulator on a local port; under a second.
#[tokio::test]
async fn an_avd_boots_and_is_driven_over_its_signed_grpc_interface() {
    let (mobile, sdk) = start(false).await;
    // `open` names the AVD and the adb serial the agent's own adb takes.
    let text = mobile.run_text(CHAT, "open", json!({ "device": "Pixel 8 API 36" })).await;
    assert_eq!(
        String::from_utf8(text.stdout).unwrap(),
        "m1  Pixel 8 API 36 · Android 16 · 705×1568 · AVD Pixel_8_API_36 · adb serial emulator-5554\n"
    );
    let opened = mobile.run(CHAT, "open", json!({ "device": "Pixel 8 API 36" })).await.json();
    assert_eq!(
        (opened["device"].as_str(), opened["nativeId"].as_str(), opened["serial"].as_str(), opened["width"].as_u64(), opened["height"].as_u64(), opened["booted"].as_bool()),
        (Some("m1"), Some(AVD), Some("emulator-5554"), Some(705), Some(1568), Some(true))
    );
    assert_eq!(sdk.heard(), ["launch Pixel_8_API_36"]);

    // The tree: the form's elements in screenshot pixels, the keyboard as
    // one element, the password's value withheld, the hidden text left out.
    let text = mobile.run_text(CHAT, "inspect", json!({ "device": "m1" })).await;
    assert_eq!(
        String::from_utf8(text.stdout).unwrap(),
        "Device: m1 · Pixel 8 API 36 · portrait · 705×1568\n\n\
         - text field \"Email\" [ref=e1] id=\"email\" [focused] [65,523 575×65]\n\
         - text field \"Password\" [ref=e2] id=\"password\" [protected] [65,601 575×65]\n\
         - button \"Sign Up\" [ref=e3] id=\"sign_up\" [65,653 261×66]\n\
         - keyboard \"Keyboard\" [ref=e4] [0,1111 141×65]\n"
    );

    mobile.run(CHAT, "tap", json!({ "device": "m1", "at": ["e3"] })).await.json();
    mobile.run(CHAT, "tap", json!({ "device": "m1", "at": ["353", "784"] })).await.json();
    mobile.run(CHAT, "type", json!({ "device": "m1", "text": "Al!" })).await.json();
    mobile.run(CHAT, "type", json!({ "device": "m1", "text": "Grüße", "into": "e1", "replace": true })).await.json();
    mobile.run(CHAT, "key", json!({ "device": "m1", "key": "back" })).await.json();
    mobile.run(CHAT, "key", json!({ "device": "m1", "key": "lock" })).await.json();
    let dark = mobile.run(CHAT, "appearance", json!({ "device": "m1", "appearance": "toggle" })).await.json();
    assert_eq!(dark["appearance"], "dark");
    let turned = mobile.run(CHAT, "rotate", json!({ "device": "m1", "orientation": "left" })).await.json();
    assert_eq!((turned["orientation"].as_str(), turned["width"].as_u64()), (Some("landscape-left"), Some(1568)));
    let heard: Vec<String> = sdk.heard().into_iter().skip(1).collect();
    assert_eq!(
        heard,
        [
            // Sign Up's centre, and screenshot pixel 353,784, in panel pixels.
            "touch 300,1050 down",
            "touch 300,1050 up",
            "touch 541,1200 down",
            "touch 541,1200 up",
            // A, l, ! as a US keyboard types them.
            "usb 0xe1 keydown",
            "usb 0x4 keydown",
            "usb 0x4 keyup",
            "usb 0xe1 keyup",
            "usb 0xf keydown",
            "usb 0xf keyup",
            "usb 0xe1 keydown",
            "usb 0x1e keydown",
            "usb 0x1e keyup",
            "usb 0xe1 keyup",
            // --replace focuses the field, then sets its text.
            "touch 540,850 down",
            "touch 540,850 up",
            "set text Grüße",
            "key GoBack",
            "power",
            "night true",
            "rotate 90",
        ]
    );

    let closed = mobile.run(CHAT, "close", json!({ "device": "m1" })).await.json();
    assert_eq!(closed["shutDown"], true);
    assert_eq!(sdk.heard().last().map(String::as_str), Some("vm SHUTDOWN"));
    assert!(sdk.heard().contains(&"guest closed".to_owned()));
}

/// A device held sideways shows its screen sideways, and a tap at a point
/// of that picture reaches the emulator at the same place on the glass,
/// in the panel's own portrait pixels, whether the app stays portrait, as
/// a launcher does, or turns with the device, as Settings does: Sign Up's
/// centre is panel pixel 300,1050 either way. Planted defects this
/// catches: the touch sent in the display's pixels, which lands elsewhere
/// on a turned display, or the turn taken the other way round. A stub
/// emulator, under a second.
#[tokio::test]
async fn a_sideways_device_is_tapped_where_its_picture_shows() {
    let (mobile, sdk) = start(true).await;
    mobile.open(CHAT, AVD).await;
    mobile.run(CHAT, "rotate", json!({ "device": "m1", "orientation": "left" })).await.json();

    // The launcher stays portrait: the form shows sideways.
    let tree = mobile.run(CHAT, "inspect", json!({ "device": "m1" })).await.json();
    assert_eq!((tree["width"].as_u64(), tree["height"].as_u64()), (Some(1568), Some(705)));
    let sign_up = reference(&tree["elements"], "Sign Up").unwrap();
    let frame = |label: &str| {
        fn find(elements: &Value, label: &str) -> Option<Value> {
            elements.as_array()?.iter().find_map(|each| if each["label"] == label { Some(each["frame"].clone()) } else { find(&each["children"], label) })
        }
        find(&tree["elements"], label).unwrap()
    };
    assert_eq!(frame("Sign Up"), json!({ "x": 653, "y": 379, "width": 66, "height": 261 }));
    mobile.run(CHAT, "tap", json!({ "device": "m1", "at": [sign_up] })).await.json();

    // The app turns with the device: the same point is the same place.
    *sdk.guest.display.lock().unwrap() = 1;
    mobile.run(CHAT, "tap", json!({ "device": "m1", "at": ["686", "509"] })).await.json();

    let touches: Vec<String> = sdk.heard().into_iter().filter(|line| line.ends_with("down")).collect();
    assert_eq!(touches, ["touch 300,1050 down", "touch 300,1050 down"]);
}

/// The screenshot is the size `open`, `inspect` and `rotate` print, upright
/// and sideways, for a Pixel 10's 1080 × 2424 screen that the emulator
/// states only in the list emulators up to at least 36.6 send: the size the
/// emulator's own rule gives, 698 × 1568, drawn by the emulator in one call;
/// and the screenshot's far corner is tapped at the panel's. An emulator
/// that draws another size is scaled here to the same size. Planted
/// defects this catches: the screen's size guessed when the emulator's map
/// is empty (1080 × 2400, printed 706 wide beside a 698-pixel picture), a
/// size rounded rather than cut down (699 beside 698), a box the emulator
/// fits otherwise (698 × 1566), or the emulator's picture passed on at a
/// size the coordinates are not in. A stub emulator, under a second.
#[tokio::test]
async fn the_screenshot_is_the_size_every_coordinate_is_in() {
    let (mobile, sdk) = start(true).await;
    *sdk.emulator.panel.lock().unwrap() = (1080, 2424);
    *sdk.emulator.listed.lock().unwrap() = true;
    let opened = mobile.run(CHAT, "open", json!({ "device": AVD })).await.json();
    let size = |answer: &Value| (answer["width"].as_u64().unwrap(), answer["height"].as_u64().unwrap());
    let screenshot = async || {
        let png = mobile.run(CHAT, "screenshot", json!({ "device": "m1" })).await.stdout;
        let reader = image::ImageReader::new(std::io::Cursor::new(png)).with_guessed_format().unwrap();
        let (width, height) = reader.into_dimensions().unwrap();
        (u64::from(width), u64::from(height))
    };
    assert_eq!(size(&opened), (698, 1568));
    for (orientation, printed, asked, corner, glass) in [
        ("portrait", (698, 1568), "screenshot PNG 1080x1568", ["697", "1567"], "touch 1078,2422 down"),
        ("left", (1568, 698), "screenshot PNG 1568x1080", ["1567", "697"], "touch 2,2422 down"),
    ] {
        let turned = mobile.run(CHAT, "rotate", json!({ "device": "m1", "orientation": orientation })).await.json();
        assert_eq!(size(&turned), printed, "held {orientation}");
        let from = sdk.heard().len();
        assert_eq!(screenshot().await, printed, "held {orientation}");
        assert_eq!(since(&sdk, from), [asked], "held {orientation}");
        let tree = mobile.run(CHAT, "inspect", json!({ "device": "m1" })).await.json();
        assert_eq!(size(&tree), printed, "held {orientation}");
        let from = sdk.heard().len();
        mobile.run(CHAT, "tap", json!({ "device": "m1", "at": corner })).await.json();
        assert_eq!(since(&sdk, from).first().map(String::as_str), Some(glass), "held {orientation}");
    }

    // An emulator that rounds otherwise: its whole picture, scaled here.
    *sdk.emulator.rounds_otherwise.lock().unwrap() = true;
    let from = sdk.heard().len();
    assert_eq!(screenshot().await, (1568, 698));
    assert_eq!(since(&sdk, from), ["screenshot PNG 1568x1080", "screenshot RGB888 0x0"]);
}

/// Watching an emulator streams its screen as H.264, starting from a key
/// frame with its parameter sets, in the frame the view draws for an Android
/// phone, its buttons on the right. Planted defects this catches: no key
/// frame first, or an iPhone's buttons on the left. A stub emulator and
/// OpenH264 at a small size; under a second.
#[tokio::test]
async fn the_live_view_shows_an_emulator_as_h264_in_its_drawn_frame() {
    let (mobile, _sdk) = start(true).await;
    mobile.open(CHAT, AVD).await;
    let mut page = Page::open(&mobile, CHAT).await;
    page.send(json!({ "type": "hello", "platform": "mac" })).await;
    page.send(json!({ "type": "watch", "device": "m1" })).await;
    page.send(json!({ "type": "frame", "device": "m1", "width": 400, "height": 800, "scale": 2.0 })).await;
    page.send(json!({ "type": "shown", "device": "m1", "width": 360, "height": 800 })).await;
    let LiveModuleMessage::Frame { frame, .. } = page.framed().await else { unreachable!() };
    let frame = serde_json::to_value(&frame).unwrap();
    assert_eq!(frame["kind"], "drawn");
    assert!(frame["buttons"].as_array().unwrap().iter().all(|button| button["side"] == "right"), "{frame}");
    let video = page.video().await;
    // Annex B: the parameter sets, then an IDR picture.
    let types: Vec<u8> = video
        .windows(5)
        .filter(|window| window[..4] == [0, 0, 0, 1])
        .map(|window| window[4] & 0x1f)
        .collect();
    assert_eq!(&types[..3], [7, 8, 5], "{types:?}");
}

/// The frame the live view gets for the stub's Pixel 8 in a view of 400 ×
/// 800 device pixels.
async fn live_frame(mobile: &Mobile) -> (Page, Value) {
    mobile.open(CHAT, AVD).await;
    let mut page = Page::open(mobile, CHAT).await;
    page.send(json!({ "type": "hello", "platform": "mac" })).await;
    page.send(json!({ "type": "watch", "device": "m1" })).await;
    page.send(json!({ "type": "frame", "device": "m1", "width": 400, "height": 800, "scale": 2.0 })).await;
    let LiveModuleMessage::Frame { frame, .. } = page.framed().await else { unreachable!() };
    (page, serde_json::to_value(&frame).unwrap())
}

/// The drawn frame follows the corners the emulator's system UI paints into
/// its picture: a Pixel 10's drawable reaches 196 pixels along each edge
/// where its display reports a radius of 132, through the drawable's own
/// curve; where the drawable does not read, the corner is the circle of the
/// reported radius. The points are the screen's, 2.625 pixels each.
/// Planted defects this catches: the circle drawn while the drawable reads,
/// a fixed radius, corners in pixels, or the APK read once for each
/// drawable. A stub emulator; milliseconds.
#[tokio::test]
async fn the_drawn_frame_follows_the_corners_the_emulators_system_ui_draws() {
    let (mobile, sdk) = start(true).await;
    let (_page, frame) = live_frame(&mobile).await;
    // Both drawables come from the APK they resolve to, read once.
    let read: Vec<String> = sdk.heard().into_iter().filter(|line| line.starts_with("apk files")).collect();
    assert_eq!(read, [format!("apk files {SYSTEM_UI_APK} res/drawable/rounded_corner_top.xml res/drawable/rounded_corner_bottom.xml")]);
    assert_eq!(frame["kind"], "drawn");
    let corner = &frame["corners"]["topLeft"];
    assert_eq!(corner["kind"], "curve", "{corner}");
    let points: Vec<(f64, f64)> = corner["points"]
        .as_array()
        .unwrap()
        .iter()
        .map(|point| (point["x"].as_f64().unwrap(), point["y"].as_f64().unwrap()))
        .collect();
    let near = |(x, y): (f64, f64), (to_x, to_y): (f64, f64)| (x - to_x).hypot(y - to_y) < 0.5;
    let reach = 196.0 / 2.625;
    assert!(near(points[0], (0.0, reach)) && near(*points.last().unwrap(), (reach, 0.0)), "{points:?}");
    // A point the drawable's curve passes through, (89.96, 11.51) in its
    // pixels, where the circle of the radius runs 7 points further in.
    let on_curve = (89.96 / 2.625, 11.51 / 2.625);
    assert!(points.windows(2).any(|pair| {
        let ((x1, y1), (x2, y2)) = (pair[0], pair[1]);
        let along = (((on_curve.0 - x1) * (x2 - x1) + (on_curve.1 - y1) * (y2 - y1)) / ((x2 - x1).powi(2) + (y2 - y1).powi(2))).clamp(0.0, 1.0);
        near((x1 + along * (x2 - x1), y1 + along * (y2 - y1)), on_curve)
    }), "{points:?}");
    // The other corners are the same corner mirrored into place.
    for corner in ["topRight", "bottomRight", "bottomLeft"] {
        assert_eq!(frame["corners"][corner], frame["corners"]["topLeft"]);
    }

    let (mobile, sdk) = start(true).await;
    sdk.guest.corners.lock().unwrap().apks.clear();
    let (_page, frame) = live_frame(&mobile).await;
    assert_eq!(frame["corners"]["topLeft"], json!({ "kind": "round", "radius": 132.0 / 2.625 }));
}

/// The corners come from the APK the device resolves SystemUI's drawable
/// to: where that is an overlay the shell may not read, they are the
/// circles of the reported radius, never the curve of the same-named
/// drawable a lower layer holds. Planted defect this catches: an
/// unreadable overlay passed over for SystemUI's own drawable. A stub
/// emulator; milliseconds.
#[tokio::test]
async fn corners_come_from_the_overlay_the_device_resolves_them_to_or_are_its_radius() {
    let (mobile, sdk) = start(true).await;
    {
        let mut corners = sdk.guest.corners.lock().unwrap();
        corners.drawables = "/vendor/overlay/SystemUIGoogle__auto_generated_rro_vendor.apk";
        corners
            .apks
            .insert(SYSTEM_UI_APK, include_bytes!("fixtures/rounded_corner.pixel_10_pro.xml").to_vec());
    }
    let (_page, frame) = live_frame(&mobile).await;
    assert_eq!(frame["corners"]["topLeft"], json!({ "kind": "round", "radius": 132.0 / 2.625 }));
    let read: Vec<String> = sdk.heard().into_iter().filter(|line| line.starts_with("apk files")).collect();
    assert_eq!(read, ["apk files /vendor/overlay/SystemUIGoogle__auto_generated_rro_vendor.apk res/drawable/rounded_corner_top.xml res/drawable/rounded_corner_bottom.xml"]);
}

/// Reading the screen's corners never holds up the view: while the device
/// has not answered, the view gets the screen's size at once, without a
/// frame, and as a page does, says the size it shows the screen at, upon
/// which the first key frame comes; the viewer's typing reaches the device
/// and closing the device lets the read go; the drawn frame follows once
/// the corners are known. Planted defects this catches: the screen's size
/// sent only with the frame, which holds the picture back until the
/// corners are read; the corners read in the view's loop; or a read left
/// running after its device closed. A stub emulator; milliseconds.
#[tokio::test]
async fn the_view_goes_on_while_the_screens_corners_are_read() {
    let (mobile, sdk) = start(true).await;
    sdk.guest.held.send_replace(true);
    mobile.open(CHAT, AVD).await;
    let mut page = Page::open(&mobile, CHAT).await;
    page.send(json!({ "type": "hello", "platform": "mac" })).await;
    page.send(json!({ "type": "watch", "device": "m1" })).await;
    page.send(json!({ "type": "frame", "device": "m1", "width": 400, "height": 800, "scale": 2.0 })).await;
    let sized = tokio::time::timeout(std::time::Duration::from_secs(10), page.until("frame")).await;
    let Ok(LiveModuleMessage::Frame { points_width, points_height, frame: None, .. }) = sized else {
        panic!("the screen's size waited for the corners: {sized:?}");
    };
    // The page shows the screen at the size the frame message said.
    assert!(points_width > 0.0 && points_height > 0.0);
    page.send(json!({ "type": "shown", "device": "m1", "width": 360, "height": 800 })).await;
    let video = tokio::time::timeout(std::time::Duration::from_secs(10), page.video()).await;
    let video = video.expect("the picture waited for the corners");
    assert!(video.windows(5).any(|window| window[..4] == [0, 0, 0, 1] && window[4] & 0x1f == 5), "no key frame came");
    page.send(json!({ "type": "input", "device": "m1", "input": { "type": "text", "text": "東京" } })).await;
    sdk.emulator.heard.until("clipboard 東京").await;
    sdk.guest.held.send_replace(false);
    let framed = tokio::time::timeout(std::time::Duration::from_secs(10), page.framed()).await;
    let Ok(LiveModuleMessage::Frame { frame, .. }) = framed else { panic!("no frame followed the corners") };
    let frame = serde_json::to_value(&frame).unwrap();
    assert_eq!(frame["corners"]["topLeft"]["kind"], "curve", "{frame}");

    let (mobile, sdk) = start(true).await;
    sdk.guest.held.send_replace(true);
    mobile.open(CHAT, AVD).await;
    let mut page = Page::open(&mobile, CHAT).await;
    page.send(json!({ "type": "hello", "platform": "mac" })).await;
    page.send(json!({ "type": "watch", "device": "m1" })).await;
    page.send(json!({ "type": "frame", "device": "m1", "width": 400, "height": 800, "scale": 2.0 })).await;
    // The view answers while the read is held.
    page.send(json!({ "type": "input", "device": "m1", "input": { "type": "text", "text": "東京" } })).await;
    sdk.emulator.heard.until("clipboard 東京").await;
    let closed = tokio::time::timeout(std::time::Duration::from_secs(10), mobile.run(CHAT, "close", json!({ "device": "m1" })));
    closed.await.expect("closing waited for the corners").json();
    sdk.emulator.heard.until("corner read let go").await;
}

/// Android Studio's frame needs nothing of the corners the device reports,
/// and comes while they are still being read; a device that reports no
/// corner radius, and has no art, has square corners. Planted defects this
/// catches: the art held up by the corner reads, or a corner made up where
/// the radius is unknown. A stub emulator and a small art; under a second.
#[tokio::test]
async fn the_art_needs_no_corners_and_unknown_corners_are_square() {
    let (mobile, sdk) = start(true).await;
    sdk.guest.held.send_replace(true);
    let art = tempfile::tempdir().unwrap();
    write_art(&art.path().join("pixel_8"));
    *sdk.art.lock().unwrap() = Some(art.path().join("pixel_8"));
    let framed = tokio::time::timeout(std::time::Duration::from_secs(10), live_frame(&mobile)).await;
    let (_page, frame) = framed.expect("the art waited for the corners");
    assert_eq!(frame["kind"], "image", "{frame}");
    assert!(!sdk.heard().iter().any(|line| line.starts_with("apk files")), "{:?}", sdk.heard());

    let (mobile, sdk) = start(true).await;
    sdk.guest.corners.lock().unwrap().radius = None;
    let (_page, frame) = live_frame(&mobile).await;
    assert_eq!(frame["kind"], "drawn");
    for corner in ["topLeft", "topRight", "bottomRight", "bottomLeft"] {
        assert_eq!(frame["corners"][corner], json!({ "kind": "round", "radius": 0.0 }), "{frame}");
    }
}

/// Writes Android Studio-like art for a 108 × 240 screen into `folder`: a
/// grey frame 20 pixels wide at the sides and 25 at the top and bottom,
/// opaque over the screen too, and a mask that is clear but for a black
/// 10-pixel square at the screen's top left corner.
fn write_art(folder: &Path) {
    std::fs::create_dir_all(folder).unwrap();
    std::fs::write(
        folder.join("layout"),
        "parts {\n  device {\n    display {\n      width 108\n      height 240\n      x 0\n      y 0\n    }\n  }\n  \
         portrait {\n    background {\n      image back.png\n    }\n    foreground {\n      mask mask.png\n    }\n  }\n}\n\
         layouts {\n  portrait {\n    width 148\n    height 290\n    part1 {\n      name portrait\n      x 0\n      y 0\n    }\n    \
         part2 {\n      name device\n      x 20\n      y 25\n    }\n  }\n}\n",
    )
    .unwrap();
    image::RgbaImage::from_pixel(148, 290, image::Rgba([128, 128, 128, 255])).save(folder.join("back.png")).unwrap();
    let mut mask = image::RgbaImage::new(108, 240);
    for (x, y, pixel) in mask.enumerate_pixels_mut() {
        if x < 10 && y < 10 {
            *pixel = image::Rgba([0, 0, 0, 255]);
        }
    }
    mask.save(folder.join("mask.png")).unwrap();
}

/// Where the Host has Android Studio's art for the AVD's device, the
/// emulator wears that frame as Android Studio draws it: the frame's image
/// around the screen and never over it, the mask over the screen's corner,
/// sized to the view's room with its screen the picture's 1080 × 2400
/// pixels scaled alike, and turned with the picture. Planted defects this
/// catches: the frame's image left over the picture, the mask left out, the
/// screen placed by the art's own size, or the frame not turned. A stub
/// emulator and a 148 × 290 art; under a second.
#[tokio::test]
async fn an_emulator_wears_android_studios_frame_where_the_host_has_its_art() {
    let (mobile, sdk) = start(true).await;
    let art = tempfile::tempdir().unwrap();
    write_art(&art.path().join("pixel_8"));
    *sdk.art.lock().unwrap() = Some(art.path().join("pixel_8"));
    let (mut page, frame) = live_frame(&mobile).await;
    let LiveFrame::Image { png, width, height, screen, .. } = serde_json::from_value(frame).unwrap() else {
        panic!("the frame is not the art's");
    };
    // 148 × 290 art pixels, ten picture pixels each, fitted into 400 × 800.
    let fit = 400.0 / 1480.0;
    assert_eq!((width, height), (400, 784));
    let close = |value: f64, expected: f64| (value - expected).abs() < 0.01;
    assert!(close(screen.x, 200.0 * fit) && close(screen.y, 250.0 * fit), "{screen:?}");
    assert!(close(screen.width, 1080.0 * fit) && close(screen.height, 2400.0 * fit), "{screen:?}");
    let drawn = image::load_from_memory(&base64::engine::general_purpose::STANDARD.decode(png).unwrap()).unwrap().into_rgba8();
    assert_eq!(drawn.get_pixel(200, 392)[3], 0, "the picture shows through the screen");
    assert_eq!(drawn.get_pixel(4, 392).0, [128, 128, 128, 255], "the frame is drawn around it");
    let corner = drawn.get_pixel((screen.x + 5.0) as u32, (screen.y + 5.0) as u32).0;
    assert_eq!(corner, [0, 0, 0, 255], "the mask covers the screen's corner");

    mobile.run(CHAT, "rotate", json!({ "device": "m1", "orientation": "left" })).await.json();
    let turned = tokio::time::timeout(std::time::Duration::from_secs(10), page.framed()).await;
    let Ok(LiveModuleMessage::Frame { frame: Some(LiveFrame::Image { width, height, screen, .. }), .. }) = turned else {
        panic!("no turned frame came");
    };
    let fit = 400.0 / 2900.0;
    assert_eq!((width, height), (400, 204));
    // The screen's 25-pixel top margin is on its left now.
    assert!(close(screen.x, 250.0 * fit) && close(screen.width, 2400.0 * fit), "{screen:?}");
}

/// What reached the emulator since `from` lines were heard.
fn since(sdk: &StubSdk, from: usize) -> Vec<String> {
    sdk.heard().into_iter().skip(from).collect()
}

/// The paste of the clipboard: Control-V, Control released, then a wait
/// for the app to take it.
const PASTE: [&str; 4] = ["usb 0xe0 keydown", "usb 0x19 keydown", "usb 0x19 keyup", "usb 0xe0 keyup"];

/// Text a US keyboard cannot type goes in at the cursor through the
/// clipboard and a paste, the clipboard put back as it was after each
/// paste, the rest typed as keys. Planted defects this catches: the text
/// refused, or the clipboard left holding the agent's text. A stub
/// emulator, milliseconds.
#[tokio::test]
async fn text_a_us_keyboard_cannot_type_is_pasted_and_the_clipboard_put_back() {
    let (mobile, sdk) = start(true).await;
    mobile.open(CHAT, AVD).await;
    let from = sdk.heard().len();
    let typed = mobile.run(CHAT, "type", json!({ "device": "m1", "text": "Grüße 東京" })).await.json();
    assert_eq!(typed["characters"], 8);
    let mut expected = vec!["usb 0xe1 keydown", "usb 0xa keydown", "usb 0xa keyup", "usb 0xe1 keyup", "usb 0x15 keydown", "usb 0x15 keyup", "clipboard üß"];
    expected.extend(PASTE);
    expected.extend(["clipboard the user's copy", "usb 0x8 keydown", "usb 0x8 keyup", "usb 0x2c keydown", "usb 0x2c keyup", "clipboard 東京"]);
    expected.extend(PASTE);
    expected.push("clipboard the user's copy");
    assert_eq!(since(&sdk, from), expected);
}

/// The viewer's own typing in a layout other than a US keyboard's, and an
/// input method's composed text, reach the focused field as the characters
/// they are: a German keyboard's ü, on the key a US keyboard has [ on, is
/// pasted and its release sends nothing; 東京 from a Chinese input method
/// is pasted. Planted defects this catches: the key's US character typed
/// instead, or the composed text refused. A stub emulator and a viewer,
/// milliseconds.
#[tokio::test]
async fn the_viewers_typing_in_any_layout_or_input_method_reaches_the_field() {
    let (mobile, sdk) = start(true).await;
    mobile.open(CHAT, AVD).await;
    let mut page = Page::open(&mobile, CHAT).await;
    page.send(json!({ "type": "hello", "platform": "mac" })).await;
    page.send(json!({ "type": "watch", "device": "m1" })).await;
    let from = sdk.heard().len();
    let key = |action: &str| {
        let mut key = json!({ "type": "key", "action": action, "key": "ü", "code": "BracketLeft", "keyCode": 186, "modifiers": 0, "repeat": false, "location": 0, "altGraph": false });
        if action == "down" {
            key["text"] = json!("ü");
        }
        json!({ "type": "input", "device": "m1", "input": key })
    };
    page.send(key("down")).await;
    page.send(key("up")).await;
    page.send(json!({ "type": "input", "device": "m1", "input": { "type": "text", "text": "東京" } })).await;
    sdk.emulator.heard.until("clipboard 東京").await;
    let mut restored = sdk.emulator.heard.count.subscribe();
    let both = async {
        while since(&sdk, from).iter().filter(|line| *line == "clipboard the user's copy").count() < 2 {
            restored.changed().await.unwrap();
        }
    };
    let waited = tokio::time::timeout(std::time::Duration::from_secs(10), both).await;
    assert!(waited.is_ok(), "the clipboard was not put back after each paste: {:?}", since(&sdk, from));
    let mut expected = vec!["clipboard ü"];
    expected.extend(PASTE);
    expected.extend(["clipboard the user's copy", "clipboard 東京"]);
    expected.extend(PASTE);
    expected.push("clipboard the user's copy");
    assert_eq!(since(&sdk, from), expected);
}

/// A finger or a key the driver holds down is released when what holds it
/// fails or is cancelled: Shift after a key the emulator refuses, the
/// finger after a swipe's refused move, and the finger of a long press
/// the agent interrupts. Planted defect this catches: the release sent
/// only on success, which leaves Shift or a finger down for every later
/// input. A stub emulator, milliseconds.
#[tokio::test]
async fn a_held_finger_or_key_is_released_when_its_command_fails_or_is_cancelled() {
    let (mobile, sdk) = start(true).await;
    mobile.open(CHAT, AVD).await;

    sdk.refuse("usb 0x4 keydown");
    mobile.run(CHAT, "type", json!({ "device": "m1", "text": "A" })).await.failure();
    sdk.emulator.heard.until("usb 0xe1 keyup").await;

    // Screenshot 353,0 to 353,784: display x 541, y from 0 to 1200.
    sdk.refuse("touch 541,300 down");
    let from = sdk.heard().len();
    mobile.run(CHAT, "swipe", json!({ "device": "m1", "at": ["353", "0", "353", "784"] })).await.failure();
    sdk.emulator.heard.until("touch 541,300 up").await;
    assert_eq!(since(&sdk, from).last().map(String::as_str), Some("touch 541,300 up"));

    let (mut input, output) = mobile
        .begin(CHAT, "tap", json!({ "device": "m1", "at": ["353", "784"], "long": true }), demi_command_protocol::CommandCaller::agent(1))
        .await;
    sdk.emulator.heard.until("touch 541,1200 down").await;
    input.cancel();
    drop(output);
    sdk.emulator.heard.until("touch 541,1200 up").await;
}

/// While another UI automation client, such as Appium, holds the
/// emulator's, a command that needs the tree fails saying so and sends
/// nothing. Planted defect this catches: the refusal reported as a broken
/// helper, or a touch sent anyway. A stub emulator, milliseconds.
#[tokio::test]
async fn inspecting_while_another_client_holds_the_ui_automation_says_so() {
    let (mobile, sdk) = start(true).await;
    mobile.open(CHAT, AVD).await;
    *sdk.guest.unreadable.lock().unwrap() = Some(demi_mobile::android::guest::TAKEN.to_owned());
    let from = sdk.heard().len();
    let (_, message) = mobile.run(CHAT, "inspect", json!({ "device": "m1" })).await.failure();
    assert!(message.contains("such as Appium, holds this emulator's UI automation"), "{message}");
    assert!(since(&sdk, from).iter().all(|line| !line.starts_with("touch")), "{:?}", since(&sdk, from));
}

/// An element is tapped at its own place on the glass in every
/// orientation, whether the app turns with the device or stays portrait:
/// Sign Up's centre, display pixel 300,1050, is that panel pixel on a
/// portrait display, and turned with the display otherwise; the emulator's
/// touches take the panel's own pixels. Planted defect this catches: a
/// turn taken the wrong way round for one orientation, or a touch sent in
/// display pixels. A stub emulator, under a second.
#[tokio::test]
async fn an_element_is_tapped_at_its_place_on_the_glass_in_every_orientation() {
    let (mobile, sdk) = start(true).await;
    mobile.open(CHAT, AVD).await;
    for orientation in ["portrait", "left", "right"] {
        mobile.run(CHAT, "rotate", json!({ "device": "m1", "orientation": orientation })).await.json();
        let picture = *sdk.emulator.rotation.borrow();
        for display in [0, picture] {
            *sdk.guest.display.lock().unwrap() = display;
            let tree = mobile.run(CHAT, "inspect", json!({ "device": "m1" })).await.json();
            let sign_up = reference(&tree["elements"], "Sign Up").unwrap();
            let from = sdk.heard().len();
            mobile.run(CHAT, "tap", json!({ "device": "m1", "at": [sign_up] })).await.json();
            let glass = match display {
                0 => "300,1050",
                1 => "30,300",
                _ => "1050,2100",
            };
            assert_eq!(
                since(&sdk, from),
                [format!("touch {glass} down"), format!("touch {glass} up")],
                "held {orientation}, the display turned {display}"
            );
        }
    }
}

/// An emulator whose pictures stop while it runs is let go as a lost
/// device: what Demi started inside it ends, and the emulator stays up and
/// Demi's, for the hour. Planted defects this catches: the helper left
/// running inside the emulator, or the emulator shut down for it. A stub
/// emulator, milliseconds.
#[tokio::test]
async fn a_lost_picture_stream_lets_the_helper_go_and_leaves_the_emulator_up() {
    let (mobile, sdk) = start(false).await;
    mobile.open(CHAT, AVD).await;
    sdk.emulator.pictures_lost.cancel();
    sdk.emulator.heard.until("guest closed").await;
    assert!(!sdk.heard().iter().any(|line| line.starts_with("vm")), "{:?}", sdk.heard());
    let listed = mobile.run(CHAT, "list", json!({})).await.json();
    let avd = listed["devices"].as_array().unwrap().iter().find(|each| each["nativeId"] == AVD).unwrap().clone();
    assert_eq!((avd["state"].as_str(), avd.get("open")), (Some("booted"), None), "{avd}");
    assert!(std::fs::read_to_string(mobile.home().join("mobile/booted.json")).unwrap().contains(AVD));
}

/// An emulator Demi started is Demi's from the moment its process runs: a
/// boot that fails, as one that never says where it listens, ends that
/// process by its id and start and leaves no record; one whose process
/// does not end stays in the record, to be shut down later. Planted
/// defects this catches: the record written only once the boot finished,
/// or a failed launch left running. A stub SDK, milliseconds.
#[tokio::test]
async fn a_failed_boot_ends_the_process_it_started_or_stays_demis() {
    let (mobile, sdk) = start(false).await;
    *sdk.silent.lock().unwrap() = true;
    let (_, message) = mobile.run(CHAT, "open", json!({ "device": AVD })).await.failure();
    assert!(message.contains("did not start in 120 s"), "{message}");
    assert_eq!(sdk.heard(), ["launch Pixel_8_API_36", "terminate 4242 1000"]);
    let record = mobile.home().join("mobile/booted.json");
    assert!(!std::fs::read_to_string(&record).unwrap_or_default().contains(AVD));

    let (mobile, sdk) = start(false).await;
    *sdk.silent.lock().unwrap() = true;
    *sdk.unkillable.lock().unwrap() = true;
    mobile.run(CHAT, "open", json!({ "device": AVD })).await.failure();
    let record = mobile.home().join("mobile/booted.json");
    let kept: Value = serde_json::from_str(&std::fs::read_to_string(&record).unwrap()).unwrap();
    assert_eq!(kept["simulators"], json!([{ "platform": "android", "nativeId": AVD, "boot": 1000 }]));
    // The list says it is still booting, so the hour's shutdown ends it.
    let listed = mobile.run(CHAT, "list", json!({})).await.json();
    let avd = listed["devices"].as_array().unwrap().iter().find(|each| each["nativeId"] == AVD).unwrap().clone();
    assert_eq!(avd["state"], "booting");
}

/// The end of an emulator's process, which is not this program's child, is
/// heard as it happens; ending one by its id and start ends it, and leaves
/// alone another process the id names now. Planted defects this catches:
/// an end that only a later file change reveals, which never comes, or a
/// signal sent to whatever process has the id. `sleep` processes, half a
/// second.
#[tokio::test]
async fn an_emulators_process_is_ended_and_its_end_heard_as_it_happens() {
    let mut brief = tokio::process::Command::new("sleep").arg("0.5").spawn().unwrap();
    let process = Process::of(brief.id().unwrap()).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), process.ended())
        .await
        .expect("the process's end is heard as it ends");
    let _reaped = brief.wait().await;

    let mut ended = tokio::process::Command::new("sleep").arg("60").spawn().unwrap();
    let process = Process::of(ended.id().unwrap()).unwrap();
    process.end().await.unwrap();
    let status = tokio::time::timeout(std::time::Duration::from_secs(10), ended.wait()).await.unwrap().unwrap();
    assert!(!status.success(), "the process ran to its end: {status}");

    let mut second = tokio::process::Command::new("sleep").arg("60").spawn().unwrap();
    let running = Process::of(second.id().unwrap()).unwrap();
    let other = Process { started: running.started - 1, ..running };
    other.end().await.unwrap();
    assert!(running.alive(), "a process that is not the one started was ended");
    second.kill().await.unwrap();
}

/// The adb server as the stub serves it, for the helper's own start: each
/// shell command is heard and answered by the test; the helper's process
/// prints `helper` and runs until the test ends; the helper's port answers
/// its JSON-RPC as a helper on an upright display does.
struct StubAdb {
    port: u16,
    commands: Arc<Mutex<Vec<String>>>,
    _server: tokio_util::task::AbortOnDropHandle<()>,
}

impl StubAdb {
    async fn start(answer: impl Fn(&str) -> String + Send + Sync + 'static, helper: &'static str) -> Self {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let commands = Arc::new(Mutex::new(Vec::new()));
        let answer = Arc::new(answer);
        let heard = commands.clone();
        let server = tokio_util::task::AbortOnDropHandle::new(tokio::spawn(async move {
            let mut connections = tokio::task::JoinSet::new();
            while let Ok((mut stream, _)) = listener.accept().await {
                let (answer, heard) = (answer.clone(), heard.clone());
                connections.spawn(async move {
                    loop {
                        let mut length = [0u8; 4];
                        if stream.read_exact(&mut length).await.is_err() {
                            return;
                        }
                        let length = usize::from_str_radix(std::str::from_utf8(&length).unwrap(), 16).unwrap();
                        let mut request = vec![0u8; length];
                        stream.read_exact(&mut request).await.unwrap();
                        let request = String::from_utf8(request).unwrap();
                        stream.write_all(b"OKAY").await.unwrap();
                        if request.starts_with("host:transport:") {
                            continue;
                        }
                        if let Some(command) = request.strip_prefix("shell:") {
                            heard.lock().unwrap().push(command.to_owned());
                            if command.contains("exec app_process") {
                                stream.write_all(helper.as_bytes()).await.unwrap();
                                // The helper runs while its connection does.
                                let _closed = stream.read(&mut [0u8; 1]).await;
                            } else {
                                stream.write_all(answer(command).as_bytes()).await.unwrap();
                            }
                            return;
                        }
                        assert_eq!(request, "tcp:9018", "a service the stub has not");
                        // One JSON-RPC call over HTTP/1.1.
                        let mut asked = Vec::new();
                        let mut byte = [0u8; 1];
                        while !asked.ends_with(b"\r\n\r\n") {
                            if stream.read_exact(&mut byte).await.is_err() {
                                return;
                            }
                            asked.push(byte[0]);
                        }
                        let headers = String::from_utf8_lossy(&asked).to_lowercase();
                        let length: usize = headers
                            .lines()
                            .find_map(|line| line.strip_prefix("content-length:"))
                            .map_or(0, |value| value.trim().parse().unwrap());
                        let mut body = vec![0u8; length];
                        stream.read_exact(&mut body).await.unwrap();
                        let call: Value = serde_json::from_slice(&body).unwrap();
                        heard.lock().unwrap().push(format!("rpc {}", call["method"].as_str().unwrap()));
                        let reply = json!({ "jsonrpc": "2.0", "id": call["id"], "result": "" }).to_string();
                        let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{reply}", reply.len());
                        let _written = stream.write_all(response.as_bytes()).await;
                        return;
                    }
                });
            }
        }));
        Self { port, commands, _server: server }
    }

    fn commands(&self) -> Vec<String> {
        self.commands.lock().unwrap().clone()
    }
}

/// What a helper prints as it starts and listens.
const LISTENS: &str = "[server] INFO: http server listening on *:9018\n";

/// The helper's shell answers with a pid file that names process 777,
/// whose environment is `environment`.
fn left_by(environment: &'static str) -> impl Fn(&str) -> String + Send + Sync + 'static {
    move |command| match command {
        "cat /data/local/tmp/demi-u2.pid 2>/dev/null" => "777\n".into(),
        "cat /proc/777/environ 2>/dev/null" => environment.into(),
        command if command.contains("&& echo present") => "present\n".into(),
        _ => String::new(),
    }
}

/// The helper an earlier program left is stopped before Demi's starts, but
/// only when the pid its file names is still that helper: an emulator
/// restarted since may give the id to any process. Planted defect this
/// catches: the pid file's process killed unchecked. A stub adb server,
/// milliseconds.
#[tokio::test]
async fn a_stale_pid_file_stops_only_demis_own_helper() {
    use demi_mobile::android::{Adb, guest::U2};
    let foreign = StubAdb::start(left_by("PATH=/system/bin\0CLASSPATH=/data/local/tmp/other.jar\0"), LISTENS).await;
    let guest = U2::new(Adb { port: foreign.port }, "emulator-5554".into());
    assert_eq!(guest.clipboard().await.unwrap(), "");
    assert!(!foreign.commands().contains(&"kill 777".to_owned()), "{:?}", foreign.commands());

    let left = StubAdb::start(left_by("PATH=/system/bin\0CLASSPATH=/data/local/tmp/demi-u2-0b74e83c55f4.jar\0"), LISTENS).await;
    let guest = U2::new(Adb { port: left.port }, "emulator-5554".into());
    assert_eq!(guest.clipboard().await.unwrap(), "");
    let commands = left.commands();
    let kill = commands.iter().position(|command| command == "kill 777").expect("Demi's own helper is stopped");
    let start = commands.iter().position(|command| command.contains("exec app_process")).unwrap();
    assert!(kill < start, "{commands:?}");
}

/// While another UI automation client, such as Appium, holds the
/// emulator's, Demi's helper cannot start, and what needs it fails saying
/// so: Android refuses a second client rather than handing it over, so
/// Demi takes the connection back only once the other client let it go.
/// Planted defect this catches: the helper's start-up failure reported as
/// a broken helper. A stub adb server, milliseconds.
#[tokio::test]
async fn the_helper_says_when_another_client_holds_the_ui_automation() {
    use demi_mobile::android::{Adb, guest::U2};
    let held = StubAdb::start(
        left_by(""),
        "java.lang.IllegalStateException: UiAutomationService android.accessibilityservice.IAccessibilityServiceClient$Stub$Proxy@c0ef88falready registered!\n",
    )
    .await;
    let guest = U2::new(Adb { port: held.port }, "emulator-5554".into());
    let refused = tokio::time::timeout(std::time::Duration::from_secs(10), guest.clipboard())
        .await
        .expect("the refusal is reported as the helper prints it")
        .unwrap_err()
        .to_string();
    assert_eq!(refused, demi_mobile::android::guest::TAKEN);
}

/// The encoder is Cisco's build, downloaded as it is first needed and
/// used only when it matches its pinned hash: a file the server sends
/// that does not is refused, kept nowhere, and asked for again next time.
/// Planted defect this catches: the download used unchecked. A stub server
/// on a local port, milliseconds.
#[tokio::test]
async fn an_encoder_that_is_not_ciscos_build_is_refused() {
    use demi_mobile::android::{Codec, openh264::Cisco};
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    let mut compressed = Vec::new();
    std::io::Read::read_to_end(&mut bzip2::read::BzEncoder::new(&b"not Cisco's library"[..], bzip2::Compression::fast()), &mut compressed).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let asked = Arc::new(Mutex::new(Vec::new()));
    let heard = asked.clone();
    let _server = tokio_util::task::AbortOnDropHandle::new(tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let mut request = vec![0u8; 4096];
            let read = stream.read(&mut request).await.unwrap();
            let line = String::from_utf8_lossy(&request[..read]).lines().next().unwrap_or_default().to_owned();
            heard.lock().unwrap().push(line);
            let header = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", compressed.len());
            stream.write_all(header.as_bytes()).await.unwrap();
            stream.write_all(&compressed).await.unwrap();
        }
    }));
    let directory = tempfile::tempdir().unwrap();
    let cisco = Cisco::from_origin(directory.path().to_owned(), origin);
    for _ in 0..2 {
        let refused = cisco.library().await.err().expect("a library that is not Cisco's is refused").to_string();
        assert!(refused.contains("does not match its pinned hash"), "{refused}");
    }
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0, "the refused file was kept");
    let asked = asked.lock().unwrap().clone();
    assert_eq!(asked.len(), 2);
    assert!(asked.iter().all(|line| line.starts_with("GET /libopenh264-2.6.0-") && line.contains(".bz2 ")), "{asked:?}");
    assert_eq!(asked[0], asked[1], "the second use asks for the same file again");
}

/// An emulator's display that turns off, by the panel's Lock here, is told
/// to the view as Android announces it, and the viewer's Wake turns it on
/// with the wake key, never the power key, which would turn a lit one off.
/// Planted defects this catches: Lock sent as the emulator's own Power key,
/// which an Android 16 guest ignores; a display that went dark with the
/// view showing its picture as though it were on; and Wake sent as the
/// power key. A stub emulator, milliseconds.
#[tokio::test]
async fn the_view_says_when_an_emulators_display_turns_off_and_wake_turns_it_on() {
    let (mobile, sdk) = start(true).await;
    mobile.open(CHAT, AVD).await;
    let mut page = Page::open(&mobile, CHAT).await;
    page.send(json!({ "type": "hello", "platform": "mac" })).await;
    page.send(json!({ "type": "watch", "device": "m1" })).await;
    let on = |message: LiveModuleMessage| serde_json::to_value(message).unwrap()["on"].as_bool().unwrap();
    assert!(on(screen(&mut page).await));
    mobile.run(CHAT, "key", json!({ "device": "m1", "key": "lock" })).await.json();
    assert!(!on(screen(&mut page).await), "Lock turned the display off");
    let before = sdk.heard().len();
    page.send(json!({ "type": "wake", "device": "m1" })).await;
    assert!(on(screen(&mut page).await), "Wake turned it on");
    assert_eq!(since(&sdk, before), ["wake"], "Wake is the wake key, not Power");
}

/// The display's state stays true after the log that tells it ends while
/// the device is open, as when adb lets go: the driver follows the log
/// again from the state read anew, so a display turned off meanwhile is
/// told to the view. Planted defect this catches: a follower that ends
/// with its log and leaves the last state standing. A stub emulator,
/// milliseconds.
#[tokio::test]
async fn the_view_hears_the_display_after_its_log_ends() {
    let (mobile, sdk) = start(true).await;
    mobile.open(CHAT, AVD).await;
    let mut page = Page::open(&mobile, CHAT).await;
    page.send(json!({ "type": "hello", "platform": "mac" })).await;
    page.send(json!({ "type": "watch", "device": "m1" })).await;
    let on = |message: LiveModuleMessage| serde_json::to_value(message).unwrap()["on"].as_bool().unwrap();
    assert!(on(screen(&mut page).await));
    // The log ends, and the display turns off on the device afterwards.
    let log = std::mem::replace(&mut *sdk.guest.log.lock().unwrap(), CancellationToken::new());
    log.cancel();
    sdk.guest.power().await.unwrap();
    assert!(!on(screen(&mut page).await), "the display turned off after its log ended");
}

/// An emulator whose display cannot be followed does not open, and what
/// the opening started ends with it: its picture stream and what Demi
/// runs in the guest. Planted defect this catches: the picture watcher
/// left running for a device that never opened. A stub emulator,
/// milliseconds.
#[tokio::test]
async fn an_emulator_that_fails_to_open_leaves_nothing_running() {
    let (mobile, sdk) = start(true).await;
    *sdk.guest.log_refused.lock().unwrap() = Some("adb is gone".into());
    let (_, message) = mobile.run(CHAT, "open", json!({ "device": AVD })).await.failure();
    assert!(message.contains("adb is gone"), "{message}");
    let mut streams = sdk.emulator.streams.subscribe();
    tokio::time::timeout(std::time::Duration::from_secs(10), streams.wait_for(|count| *count == 0))
        .await
        .expect("the picture stream ends with the failed open")
        .unwrap();
    assert!(sdk.heard().iter().any(|line| line == "guest closed"), "{:?}", sdk.heard());
}

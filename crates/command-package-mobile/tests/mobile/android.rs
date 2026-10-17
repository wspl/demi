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

use demi_command_package_mobile_protocol::{live::LiveModuleMessage, mobile::PowerState};
use demi_mobile::{
    android::{
        Avd, Discovery, Guest, Process, Running, Sdk,
        proto::{
            EmulatorStatus, Image, ImageFormat, KeyboardEvent, Notification, PhysicalModelValue, Rotation,
            TouchEvent, VmRunState, emulator_controller_server::{EmulatorController, EmulatorControllerServer},
            keyboard_event::KeyEventType, notification, rotation::SkinRotation,
        },
    },
    driver::Result,
    testing::FakeDriver,
};

use crate::{CHAT, Mobile, live::Page};

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

    fn picture(rotation: u8, width: u32, height: u32) -> Image {
        // The picture's size within the most asked for, turned as held.
        let (panel_width, panel_height) = if rotation % 2 == 1 { (2400, 1080) } else { (1080, 2400) };
        let factor = (f64::from(width.max(1)) / f64::from(panel_width)).min(f64::from(height.max(1)) / f64::from(panel_height)).min(1.0);
        let (width, height) = ((f64::from(panel_width) * factor) as u32, (f64::from(panel_height) * factor) as u32);
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
        let config = [("hw.lcd.width", "1080"), ("hw.lcd.height", "2400"), ("hw.lcd.density", "420")];
        Ok(tonic::Response::new(EmulatorStatus {
            booted: true,
            platform_config: config.into_iter().map(|(key, value)| (key.into(), value.into())).collect(),
            ..EmulatorStatus::default()
        }))
    }

    async fn get_screenshot(&self, request: tonic::Request<ImageFormat>) -> std::result::Result<tonic::Response<Image>, tonic::Status> {
        self.signed(&request)?;
        let asked = request.into_inner();
        let mut image = StubEmulator::picture(*self.rotation.borrow(), asked.width, asked.height);
        image.image = Bytes::from_static(b"\x89PNG\r\n\x1a\n");
        Ok(tonic::Response::new(image))
    }

    type streamScreenshotStream = Streamed<Image>;

    async fn stream_screenshot(&self, request: tonic::Request<ImageFormat>) -> std::result::Result<tonic::Response<Streamed<Image>>, tonic::Status> {
        self.signed(&request)?;
        let asked = request.into_inner();
        let (sender, pictures) = mpsc::unbounded_channel();
        let mut drawn = self.drawn.subscribe();
        let rotation = self.rotation.subscribe();
        let ended = self.ended.clone();
        let lost = self.pictures_lost.clone();
        tokio::spawn(async move {
            loop {
                drawn.borrow_and_update();
                if sender.send(Ok(StubEmulator::picture(*rotation.borrow(), asked.width, asked.height))).is_err() {
                    return;
                }
                tokio::select! {
                    () = ended.cancelled() => return,
                    () = lost.cancelled() => return,
                    changed = drawn.changed() => if changed.is_err() { return },
                }
            }
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
}

/// What the stub's app shows and holds: the focused Email field's text,
/// which a paste adds the clipboard to, and the clipboard.
#[derive(Default)]
struct Screen {
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
                Avd { name: "Medium_Phone_API_35".into(), display: "Medium Phone API 35".into(), model: "Medium Phone".into(), os: "Android 15".into() },
                Avd { name: AVD.into(), display: "Pixel 8 API 36".into(), model: "Pixel 8".into(), os: "Android 16".into() },
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
        "m1  Pixel 8 API 36 · Android 16 · 706×1568 · AVD Pixel_8_API_36 · adb serial emulator-5554\n"
    );
    let opened = mobile.run(CHAT, "open", json!({ "device": "Pixel 8 API 36" })).await.json();
    assert_eq!(
        (opened["device"].as_str(), opened["nativeId"].as_str(), opened["serial"].as_str(), opened["width"].as_u64(), opened["height"].as_u64(), opened["booted"].as_bool()),
        (Some("m1"), Some(AVD), Some("emulator-5554"), Some(706), Some(1568), Some(true))
    );
    assert_eq!(sdk.heard(), ["launch Pixel_8_API_36"]);

    // The tree: the form's elements in screenshot pixels, the keyboard as
    // one element, the password's value withheld, the hidden text left out.
    let text = mobile.run_text(CHAT, "inspect", json!({ "device": "m1" })).await;
    assert_eq!(
        String::from_utf8(text.stdout).unwrap(),
        "Device: m1 · Pixel 8 API 36 · portrait · 706×1568\n\n\
         - text field \"Email\" [ref=e1] id=\"email\" [focused] [65,523 576×65]\n\
         - text field \"Password\" [ref=e2] id=\"password\" [protected] [65,601 576×65]\n\
         - button \"Sign Up\" [ref=e3] id=\"sign_up\" [65,653 262×66]\n\
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
            "touch 540,1200 down",
            "touch 540,1200 up",
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
            "key Power",
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
    assert_eq!((tree["width"].as_u64(), tree["height"].as_u64()), (Some(1568), Some(706)));
    let sign_up = reference(&tree["elements"], "Sign Up").unwrap();
    let frame = |label: &str| {
        fn find(elements: &Value, label: &str) -> Option<Value> {
            elements.as_array()?.iter().find_map(|each| if each["label"] == label { Some(each["frame"].clone()) } else { find(&each["children"], label) })
        }
        find(&tree["elements"], label).unwrap()
    };
    assert_eq!(frame("Sign Up"), json!({ "x": 653, "y": 379, "width": 66, "height": 262 }));
    mobile.run(CHAT, "tap", json!({ "device": "m1", "at": [sign_up] })).await.json();

    // The app turns with the device: the same point is the same place.
    *sdk.guest.display.lock().unwrap() = 1;
    mobile.run(CHAT, "tap", json!({ "device": "m1", "at": ["686", "510"] })).await.json();

    let touches: Vec<String> = sdk.heard().into_iter().filter(|line| line.ends_with("down")).collect();
    assert_eq!(touches, ["touch 300,1050 down", "touch 300,1050 down"]);
}

/// Watching an emulator streams its screen as H.264, starting from a key
/// frame with its parameter sets, in the outline frame of an Android
/// phone, its buttons on the right. Planted defects this catches: no key
/// frame first, or an iPhone's buttons on the left. A stub emulator and
/// OpenH264 at a small size; under a second.
#[tokio::test]
async fn the_live_view_shows_an_emulator_as_h264_in_its_outline() {
    let (mobile, _sdk) = start(true).await;
    mobile.open(CHAT, AVD).await;
    let mut page = Page::open(&mobile, CHAT).await;
    page.send(json!({ "type": "hello", "platform": "mac" })).await;
    page.send(json!({ "type": "watch", "device": "m1" })).await;
    page.send(json!({ "type": "frame", "device": "m1", "width": 400, "height": 800, "scale": 2.0 })).await;
    let LiveModuleMessage::Frame { frame, .. } = page.until("frame").await else { unreachable!() };
    let frame = serde_json::to_value(&frame).unwrap();
    assert_eq!(frame["kind"], "outline");
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

    // Screenshot 353,0 to 353,784: display x 540, y from 0 to 1200.
    sdk.refuse("touch 540,300 down");
    let from = sdk.heard().len();
    mobile.run(CHAT, "swipe", json!({ "device": "m1", "at": ["353", "0", "353", "784"] })).await.failure();
    sdk.emulator.heard.until("touch 540,300 up").await;
    assert_eq!(since(&sdk, from).last().map(String::as_str), Some("touch 540,300 up"));

    let (mut input, output) = mobile
        .begin(CHAT, "tap", json!({ "device": "m1", "at": ["353", "784"], "long": true }), demi_command_protocol::CommandCaller::agent(1))
        .await;
    sdk.emulator.heard.until("touch 540,1200 down").await;
    input.cancel();
    drop(output);
    sdk.emulator.heard.until("touch 540,1200 up").await;
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
    assert!(asked[0].starts_with("GET /libopenh264-2.6.0-mac-arm64.dylib.bz2 "), "{asked:?}");
}

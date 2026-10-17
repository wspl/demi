//! A driver the tests script (`crates-and-packages.md` § Module layout): its
//! simulators, each device's screen, tree and drawing are what the test
//! sets, and what reaches a device is logged in its points.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use bytes::Bytes;
use futures_util::future::BoxFuture;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use demi_command_package_mobile_protocol::mobile::{Appearance, Orientation, MobilePlatform, PowerState};

use crate::driver::{
    Booting, Button, Device, DeviceCapture, Driver, DriverError, Frame, FrameArea, KeyPress, Png, Points, RawElement,
    Recording, Result, Screen, Simulator, TouchPhase,
};

/// What reached the devices and the driver, in order.
pub type Log = Arc<Mutex<Vec<String>>>;

/// A platform whose simulators the test lists.
pub struct FakeDriver {
    platform: MobilePlatform,
    simulators: Mutex<Vec<Simulator>>,
    devices: Mutex<HashMap<String, Arc<FakeDevice>>>,
    /// The screen in portrait each device attaches with: points and pixels.
    portrait: (Points, (u32, u32)),
    /// The next boot's mark.
    boots: std::sync::atomic::AtomicU64,
    /// While set, a boot that started waits for the test to let it end.
    held: Mutex<Option<Arc<HeldBoots>>>,
    /// Why a shutdown fails, while it does.
    shutdown_fails: Mutex<Option<String>>,
    pub log: Log,
}

impl FakeDriver {
    /// An iOS driver with `simulators`, whose screens are a 393 × 852 point
    /// iPhone at 3 pixels a point.
    pub fn ios(simulators: Vec<Simulator>) -> Arc<Self> {
        Arc::new(Self {
            platform: MobilePlatform::Ios,
            simulators: Mutex::new(simulators),
            devices: Mutex::default(),
            portrait: (Points { x: 393.0, y: 852.0 }, (1179, 2556)),
            boots: std::sync::atomic::AtomicU64::new(100),
            held: Mutex::default(),
            shutdown_fails: Mutex::default(),
            log: Log::default(),
        })
    }

    /// Boots from now on take as long as the test lets them: each started
    /// boot has its simulator booted, says so, then waits for a permit.
    pub fn hold_boots(&self) -> Arc<HeldBoots> {
        let held = Arc::new(HeldBoots {
            started: tokio::sync::Notify::new(),
            permits: tokio::sync::Semaphore::new(0),
        });
        *self.held.lock().unwrap() = Some(held.clone());
        held
    }

    /// Shutdowns fail with `reason` from now on, or succeed again with none.
    pub fn fail_shutdowns(&self, reason: Option<&str>) {
        *self.shutdown_fails.lock().unwrap() = reason.map(str::to_owned);
    }

    /// The simulator `native_id` as the driver lists it now.
    pub fn listed(&self, native_id: &str) -> Simulator {
        self.simulators
            .lock()
            .unwrap()
            .iter()
            .find(|simulator| simulator.native_id == native_id)
            .expect("the test listed the simulator")
            .clone()
    }

    /// A simulator as `simctl` lists it; a booted one in its boot 1.
    pub fn simulator(native_id: &str, name: &str, state: PowerState) -> Simulator {
        Simulator {
            native_id: native_id.into(),
            name: name.into(),
            model: name.into(),
            os: "iOS 26.5".into(),
            boot: (state == PowerState::Booted).then_some(1),
            state,
        }
    }

    /// The user shuts `native_id` down and boots it again, outside Demi.
    pub fn reboot(&self, native_id: &str) {
        self.log(format!("user reboots {native_id}"));
        self.set_state(native_id, PowerState::Booted);
    }

    /// The device attached for `native_id`, once it is.
    pub fn device(&self, native_id: &str) -> Arc<FakeDevice> {
        self.devices.lock().unwrap()[native_id].clone()
    }

    fn log(&self, line: String) {
        self.log.lock().unwrap().push(line);
    }

    /// Puts `native_id` in `state`; a boot gets a new mark.
    fn set_state(&self, native_id: &str, state: PowerState) {
        let boot = (state == PowerState::Booted)
            .then(|| self.boots.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
        for simulator in self.simulators.lock().unwrap().iter_mut() {
            if simulator.native_id == native_id {
                simulator.state = state;
                simulator.boot = boot;
            }
        }
    }
}

impl Driver for FakeDriver {
    fn platform(&self) -> MobilePlatform {
        self.platform
    }

    fn unavailable(&self) -> Option<String> {
        None
    }

    fn list(&self) -> BoxFuture<'_, Result<Vec<Simulator>>> {
        Box::pin(async { Ok(self.simulators.lock().unwrap().clone()) })
    }

    fn boot<'a>(&'a self, native_id: &'a str, booting: &'a dyn Booting) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            self.log(format!("boot {native_id}"));
            self.set_state(native_id, PowerState::Booted);
            let boot = self.simulators.lock().unwrap().iter().find(|simulator| simulator.native_id == native_id).and_then(|simulator| simulator.boot);
            if let Some(boot) = boot {
                booting.began(boot);
            }
            let held = self.held.lock().unwrap().clone();
            if let Some(held) = held {
                held.started.notify_one();
                let _permit = held.permits.acquire().await.expect("the test keeps its gate");
                self.log(format!("booted {native_id}"));
            }
            Ok(())
        })
    }

    fn shutdown<'a>(&'a self, native_id: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            if let Some(reason) = self.shutdown_fails.lock().unwrap().clone() {
                self.log(format!("shutdown {native_id} failed"));
                return Err(DriverError::Failed(reason));
            }
            self.log(format!("shutdown {native_id}"));
            self.set_state(native_id, PowerState::ShutDown);
            Ok(())
        })
    }

    fn attach<'a>(
        &'a self,
        conversation: &'a str,
        native_id: &'a str,
    ) -> BoxFuture<'a, Result<Arc<dyn Device>>> {
        Box::pin(async move {
            self.log(format!("attach {native_id} for {conversation}"));
            let (points, pixels) = self.portrait;
            let device = Arc::new(FakeDevice {
                native_id: native_id.into(),
                screen: watch::Sender::new(Screen {
                    orientation: Orientation::Portrait,
                    points,
                    pixels,
                }),
                drawn: watch::Sender::new(0),
                ended: CancellationToken::new(),
                tree: Mutex::default(),
                appearance: Mutex::new(Appearance::Light),
                recording: Mutex::default(),
                log: self.log.clone(),
            });
            self.devices
                .lock()
                .unwrap()
                .insert(native_id.into(), device.clone());
            Ok(device as Arc<dyn Device>)
        })
    }
}

/// Boots the test holds: each one that starts says so, and ends once the
/// test adds a permit.
pub struct HeldBoots {
    pub started: tokio::sync::Notify,
    pub permits: tokio::sync::Semaphore,
}

/// A device the test scripts.
pub struct FakeDevice {
    native_id: String,
    screen: watch::Sender<Screen>,
    drawn: watch::Sender<u64>,
    ended: CancellationToken,
    tree: Mutex<Vec<RawElement>>,
    appearance: Mutex<Appearance>,
    /// The file the recording under way writes.
    recording: Mutex<Option<std::path::PathBuf>>,
    log: Log,
}

impl FakeDevice {
    /// The screen shows `tree` from now on.
    pub fn show(&self, tree: Vec<RawElement>) {
        *self.tree.lock().unwrap() = tree;
    }

    /// The device draws a new picture.
    pub fn draw(&self) {
        self.drawn.send_modify(|count| *count += 1);
    }

    /// The device goes away, as a simulator shut down by another tool.
    pub fn lose(&self) {
        self.ended.cancel();
    }

    fn log(&self, line: String) {
        self.log.lock().unwrap().push(format!("{}: {line}", self.native_id));
    }
}

fn at(points: Points) -> String {
    format!("{:.1},{:.1}", points.x, points.y)
}

impl Device for FakeDevice {
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
            let factor = (f64::from(long_side) / f64::from(width.max(height))).min(1.0);
            self.log("screenshot".into());
            Ok(Png {
                bytes: Bytes::from_static(b"\x89PNG fake"),
                width: (f64::from(width) * factor).round() as u32,
                height: (f64::from(height) * factor).round() as u32,
            })
        })
    }

    fn tree(&self) -> BoxFuture<'_, Result<Vec<RawElement>>> {
        Box::pin(async { Ok(self.tree.lock().unwrap().clone()) })
    }

    fn touch(&self, phase: TouchPhase, point: Points) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            self.log(format!("touch {phase:?} {}", at(point)));
            Ok(())
        })
    }

    fn tap(&self, point: Points, seconds: f64) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            self.log(format!("tap {} for {seconds} s", at(point)));
            Ok(())
        })
    }

    fn swipe(&self, from: Points, to: Points) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            self.log(format!("swipe {} to {}", at(from), at(to)));
            Ok(())
        })
    }

    fn press(&self, button: Button) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            self.log(format!("press {button:?}"));
            Ok(())
        })
    }

    fn key(&self, key: KeyPress) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            self.log(format!("key {:#x} {}", key.usage, if key.down { "down" } else { "up" }));
            Ok(())
        })
    }

    fn type_keys<'a>(&'a self, keys: &'a [crate::keys::Typed]) -> BoxFuture<'a, Result<()>> {
        // The log says the characters the keys type on a US keyboard.
        let text: String = keys
            .iter()
            .filter_map(|key| (' '..='~').chain(['\n', '\t']).find(|character| crate::keys::typed(*character) == Some(*key)))
            .collect();
        Box::pin(async move {
            self.log(format!("text {text:?}"));
            Ok(())
        })
    }

    fn insert<'a>(&'a self, text: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            self.log(format!("insert {text:?}"));
            // The field draws the inserted text.
            self.draw();
            Ok(())
        })
    }

    fn set_value<'a>(&'a self, point: Points, text: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            self.log(format!("set {} {text:?}", at(point)));
            Ok(())
        })
    }

    fn orient(&self, orientation: Orientation) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            self.log(format!("orient {orientation}"));
            self.screen.send_modify(|screen| {
                let turned = screen.orientation.landscape() != orientation.landscape();
                screen.orientation = orientation;
                if turned {
                    screen.points = Points {
                        x: screen.points.y,
                        y: screen.points.x,
                    };
                    screen.pixels = (screen.pixels.1, screen.pixels.0);
                }
            });
            Ok(())
        })
    }

    fn appearance(&self) -> BoxFuture<'_, Result<Appearance>> {
        Box::pin(async { Ok(*self.appearance.lock().unwrap()) })
    }

    fn set_appearance(&self, appearance: Appearance) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            self.log(format!("appearance {appearance}"));
            *self.appearance.lock().unwrap() = appearance;
            Ok(())
        })
    }

    fn record_start<'a>(&'a self, file: &'a std::path::Path) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            self.log("record start".into());
            std::fs::write(file, b"fake mp4").map_err(|error| DriverError::Failed(error.to_string()))?;
            *self.recording.lock().unwrap() = Some(file.to_owned());
            Ok(())
        })
    }

    fn record_stop(&self) -> BoxFuture<'_, Result<Recording>> {
        Box::pin(async {
            self.log("record stop".into());
            self.recording
                .lock()
                .unwrap()
                .take()
                .ok_or_else(|| DriverError::Failed("the fake device is not recording".into()))?;
            Ok(Recording { duration_ms: 1500 })
        })
    }

    /// Xcode installs no frame for a fake device: the view draws an outline
    /// around a screen whose corners have a 55-point radius.
    fn frame(&self, area: FrameArea) -> BoxFuture<'_, Result<Frame>> {
        Box::pin(async move {
            self.log(format!("frame {}x{}@{}", area.width, area.height, area.scale));
            Ok(Frame::Undrawn { corner_radius: 55.0 })
        })
    }

    fn capture(
        &self,
        _size: (u32, u32),
        _fps: u32,
        _bitrate: u32,
    ) -> BoxFuture<'_, Result<Box<dyn DeviceCapture>>> {
        Box::pin(async { Err(DriverError::Failed("the fake device captures nothing".into())) })
    }

    fn detach(&self) -> BoxFuture<'_, Result<()>> {
        Box::pin(async {
            self.log("detach".into());
            Ok(())
        })
    }
}

/// OpenH264 built from its source, which the tests encode with instead of
/// downloading Cisco's build.
pub struct BuiltOpenH264;

impl crate::android::Codec for BuiltOpenH264 {
    fn library(&self) -> BoxFuture<'_, Result<crate::android::Load>> {
        Box::pin(async {
            Ok(std::sync::Arc::new(|| Ok(openh264::OpenH264API::from_source())) as crate::android::Load)
        })
    }
}

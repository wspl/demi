//! The `mobile.*` operations: each the one
//! operation behind both a command and the panel's control. Inputs act and
//! return once the device took them; `screenshot` and `inspect` first wait
//! for a settled picture.

use demi_shared_types::output::warning;
use std::{sync::Arc, time::Duration};

use bytes::Bytes;
use serde::Serialize;
use serde_json::Value;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use demi_command_package_mobile_protocol::mobile::{
    Appearance, AppearanceChoice, AppearanceResult, CloseResult, Direction, ElementRef,
    INLINE_BYTES, InspectResult, Key, KeyResult, MobileOperation, OpenResult, Orientation,
    MobilePlatform, RecordAction, RecordResult, Rotation, RotateResult,
    ScreenshotResult, SwipeResult, TapResult, TypeResult, WAIT_SECONDS, WaitResult,
};
use demi_live_view_protocol::LivePoint;

use crate::{
    devices::{Devices, OpenDevice},
    driver::{Button, KeyPress, Points},
    error::MobileError,
    keys,
    refs::Listed,
    scale::Scale,
    text, tree,
};

/// A settled picture: no new frame from the device for this long. Neither
/// platform's own idle signal is used, since both report idle while an app
/// draws its own animation; both drivers send a frame only when the screen
/// changes, so the live view's frames tell it at no cost.
pub const QUIET: Duration = Duration::from_millis(300);
/// The longest a screenshot or an inspect waits for one.
pub const SETTLE_LIMIT: Duration = Duration::from_millis(1500);
/// How often `wait` reads the tree again.
const WAIT_INTERVAL: Duration = Duration::from_millis(250);
/// A long press.
const LONG_PRESS_SECONDS: f64 = 1.0;
/// How much of an element's side a swipe across it travels.
const SWIPE_SPAN: f64 = 0.6;

/// The invocation an operation serves.
pub struct Call<'a> {
    pub conversation: &'a str,
    /// The command's path, such as `demi mobile screenshot`, which a note
    /// on standard error starts with.
    pub command: &'a str,
    pub cwd: &'a str,
    /// Where the user's panel puts a screenshot or a recording it saves on
    /// the user's computer: the conversation's scratch directory, for a call
    /// the user started; none for the agent's (`crates/command-package-mobile/src/live.rs`).
    pub scratch: Option<std::path::PathBuf>,
    /// The conversation's scratch directory on this Host, whoever calls:
    /// where a recording is written while it runs, so that it goes with
    /// the conversation whatever becomes of the program.
    pub recordings: std::path::PathBuf,
    /// Whether the agent runs it, which the view shows; the user's panel
    /// controls are the user's, and show nothing (`crates/command-package-mobile/src/live.rs`).
    pub agent: bool,
    pub cancel: &'a CancellationToken,
}

/// What an operation produced.
pub enum Produced {
    /// A result, its JSON and its text.
    Result { json: Value, text: String },
    /// A file's bytes for standard output, a PNG, and what it holds, for
    /// standard error.
    Bytes { bytes: Bytes, about: String },
    /// A file for standard output, an MP4 too large to hold at once, which
    /// is removed once written; and what it holds, for standard error.
    File { file: std::path::PathBuf, about: String },
}

/// What an operation produced, and the notes it writes to standard error.
pub struct Outcome {
    pub produced: Produced,
    pub notes: Vec<String>,
}

impl Outcome {
    fn result(json: impl Serialize, text: String) -> Result<Self, MobileError> {
        Ok(Self {
            produced: Produced::Result {
                json: serde_json::to_value(json).map_err(|error| MobileError::Failed(error.to_string()))?,
                text,
            },
            notes: Vec::new(),
        })
    }
}

pub async fn run(
    devices: &Arc<Devices>,
    call: &Call<'_>,
    operation: MobileOperation,
) -> Result<Outcome, MobileError> {
    let open = match operation.device() {
        Some(handle) => Some(devices.get(call.conversation, handle)?),
        None => None,
    };
    // Commands on one device take turns in their lane, and each is its use.
    // The agent's uses it from its start, the wait for its turn included.
    let turn_of = open.clone();
    let _agent = match &turn_of {
        Some(open) if call.agent => Some(open.agent_uses()),
        _ => None,
    };
    let _turn = match &turn_of {
        Some(open) => {
            open.used();
            Some(turn(open, lane(&operation)).await)
        }
        None => None,
    };
    let done = async {
        match (operation, open) {
            (MobileOperation::List(_), _) => {
                let listed = devices.list(call.conversation).await?;
                let text = text::list(&listed);
                let mut outcome = Outcome::result(&listed, text)?;
                outcome.notes = listed
                    .unavailable
                    .iter()
                    .map(|unavailable| warning("demi mobile list", &unavailable.reason).trim_end().to_owned())
                    .collect();
                Ok(outcome)
            }
            (MobileOperation::Open(input), _) => open_device(devices, call, &input.device).await,
            (MobileOperation::Close(input), Some(open)) => {
                drop(open);
                let ended = devices.close(call.conversation, &input.device).await?;
                let text = match (&ended.failure, ended.shut_down) {
                    (Some(failure), _) => format!(
                        "Closed {}, but it did not shut down: {failure}. Demi shuts it down after an hour unused.\n",
                        input.device
                    ),
                    (None, true) => format!("Closed {} and shut it down.\n", input.device),
                    (None, false) => {
                        format!("Closed {}; it stays booted, as you or another tool booted it.\n", input.device)
                    }
                };
                Outcome::result(
                    CloseResult {
                        device: input.device,
                        shut_down: ended.shut_down,
                    },
                    text,
                )
            }
            (MobileOperation::Screenshot(input), Some(open)) => {
                let settled = settle(&open).await;
                let png = open.device.screenshot().await?;
                let about = format!("{} · {} · {}×{}", open.handle, open.name, png.width, png.height);
                let mut notes = Vec::new();
                if !settled {
                    notes.push(unsettled(call));
                }
                let output = match input.output {
                    Some(output) => Some((output, input.overwrite == Some(true))),
                    None => panel_file(call, &open, "png")?.map(|path| (path, false)),
                };
                let produced = match output {
                    Some((output, overwrite)) => {
                        let path = save(call.cwd, &output, overwrite, &png.bytes).await?;
                        let text = format!("Saved {about} to {path}.\n");
                        let json = ScreenshotResult {
                            device: open.handle.clone(),
                            path,
                            width: png.width,
                            height: png.height,
                            settled,
                        };
                        Outcome::result(json, text)?.produced
                    }
                    None => Produced::Bytes { bytes: png.bytes, about },
                };
                Ok(Outcome { produced, notes })
            }
            (MobileOperation::Inspect(input), Some(open)) => {
                let settled = settle(&open).await;
                let raw = open.device.tree().await?;
                let screen = *open.device.screen().borrow();
                let scale = Scale::of(&screen);
                let mut elements = {
                    let mut refs = open.refs.lock().expect("the refs are intact");
                    tree::elements(&raw, input.keyboard == Some(true), &scale, &mut refs)
                };
                let mut truncated = false;
                let result = loop {
                    let result = InspectResult {
                        device: open.handle.clone(),
                        orientation: screen.orientation,
                        width: scale.width,
                        height: scale.height,
                        settled,
                        elements: elements.clone(),
                        truncated,
                    };
                    let size = serde_json::to_vec(&result).map(|bytes| bytes.len()).unwrap_or(0);
                    if size <= INLINE_BYTES || !shorten(&mut elements) {
                        break result;
                    }
                    truncated = true;
                };
                let text = text::inspect(&result, &open.name);
                let mut outcome = Outcome::result(result, text)?;
                if !settled {
                    outcome.notes.push(unsettled(call));
                }
                Ok(outcome)
            }
            (MobileOperation::Tap(input), Some(open)) => {
                let scale = Scale::of(&open.device.screen().borrow());
                let (at, element) = match input.at.as_slice() {
                    [reference] => {
                        let listed = listed(&open, reference)?;
                        (listed.1.frame.centre(), Some(listed))
                    }
                    [x, y] => (point(&scale, x, y)?, None),
                    _ => return Err(MobileError::Usage("tap takes an element ref, or x and y".into())),
                };
                let seconds = if input.long == Some(true) { LONG_PRESS_SECONDS } else { 0.0 };
                acting(&open, call, at, open.device.tap(at, seconds)).await?;
                let point = scale.point(at);
                let summary = element.map(|(reference, listed)| summary(reference, &listed));
                let text = match &summary {
                    Some(element) => format!("Tapped {} at {},{}.\n", text::summary(element), point.x, point.y),
                    None => format!("Tapped {},{}.\n", point.x, point.y),
                };
                Outcome::result(
                    TapResult {
                        device: open.handle.clone(),
                        point,
                        element: summary,
                    },
                    text,
                )
            }
            (MobileOperation::Swipe(input), Some(open)) => {
                let scale = Scale::of(&open.device.screen().borrow());
                let (from, to, element) = match input.at.as_slice() {
                    [reference, direction] => {
                        let (reference, listed) = listed(&open, reference)?;
                        let direction: Direction = direction.parse().map_err(|_| {
                            MobileError::Usage(format!("{direction:?} is not up, down, left or right"))
                        })?;
                        let (from, to) = across(&listed, direction);
                        (from, to, Some(summary(reference, &listed)))
                    }
                    [x1, y1, x2, y2] => (point(&scale, x1, y1)?, point(&scale, x2, y2)?, None),
                    _ => {
                        return Err(MobileError::Usage(
                            "swipe takes x1 y1 x2 y2, or an element ref and up, down, left or right".into(),
                        ));
                    }
                };
                acting(&open, call, to, open.device.swipe(from, to)).await?;
                let (start, end) = (scale.point(from), scale.point(to));
                let text = match &element {
                    Some(element) => format!(
                        "Swiped across {} from {},{} to {},{}.\n",
                        text::summary(element),
                        start.x,
                        start.y,
                        end.x,
                        end.y
                    ),
                    None => format!("Swiped from {},{} to {},{}.\n", start.x, start.y, end.x, end.y),
                };
                Outcome::result(
                    SwipeResult {
                        device: open.handle.clone(),
                        from: start,
                        to: end,
                        element,
                    },
                    text,
                )
            }
            (MobileOperation::Type(input), Some(open)) => {
                let replace = input.replace == Some(true);
                let target = match &input.into {
                    Some(reference) => Some(listed(&open, reference.as_str())?),
                    None if replace => {
                        return Err(MobileError::Usage("--replace sets a field's text: name it with --into".into()));
                    }
                    None => None,
                };
                let characters = input.text.chars().count();
                match &target {
                    Some((_, listed)) if replace => {
                        let at = listed.frame.centre();
                        acting(&open, call, at, open.device.set_value(at, &input.text)).await?;
                    }
                    Some((_, listed)) => {
                        let at = listed.frame.centre();
                        acting(&open, call, at, open.device.tap(at, 0.0)).await?;
                        keys::enter(&open.device, &input.text).await?;
                    }
                    None => keys::enter(&open.device, &input.text).await?,
                }
                let element = target.map(|(reference, listed)| summary(reference, &listed));
                let verb = if replace { "Set" } else { "Typed" };
                let text = match &element {
                    Some(element) => format!("{verb} {characters} characters into {}.\n", text::summary(element)),
                    None => format!("{verb} {characters} characters.\n"),
                };
                Outcome::result(
                    TypeResult {
                        device: open.handle.clone(),
                        characters,
                        replaced: replace,
                        element,
                    },
                    text,
                )
            }
            (MobileOperation::Key(input), Some(open)) => {
                press(&open, input.key).await?;
                Outcome::result(
                    KeyResult {
                        device: open.handle.clone(),
                        key: input.key,
                    },
                    format!("Pressed {}.\n", input.key),
                )
            }
            (MobileOperation::Rotate(input), Some(open)) => {
                let current = open.device.screen().borrow().orientation;
                let next = match input.orientation {
                    Rotation::Portrait => Orientation::Portrait,
                    Rotation::Landscape if current.landscape() => current,
                    Rotation::Landscape => Orientation::LandscapeLeft,
                    Rotation::Left => current.left(),
                    Rotation::Right => current.right(),
                };
                open.device.orient(next).await?;
                let scale = Scale::of(&open.device.screen().borrow());
                let text = format!(
                    "Turned {} {}; screenshots are {}×{}.\n",
                    open.handle,
                    text::orientation(next),
                    scale.width,
                    scale.height
                );
                Outcome::result(
                    RotateResult {
                        device: open.handle.clone(),
                        orientation: next,
                        width: scale.width,
                        height: scale.height,
                    },
                    text,
                )
            }
            (MobileOperation::Appearance(input), Some(open)) => {
                let appearance = match input.appearance {
                    AppearanceChoice::Light => Appearance::Light,
                    AppearanceChoice::Dark => Appearance::Dark,
                    AppearanceChoice::Toggle => match open.device.appearance().await? {
                        Appearance::Light => Appearance::Dark,
                        Appearance::Dark => Appearance::Light,
                    },
                };
                open.device.set_appearance(appearance).await?;
                Outcome::result(
                    AppearanceResult {
                        device: open.handle.clone(),
                        appearance,
                    },
                    format!("{} is {appearance} now.\n", open.handle),
                )
            }
            (MobileOperation::Wait(input), Some(open)) => wait(&open, input).await,
            (MobileOperation::Record(input), Some(open)) => match input.action {
                RecordAction::Start => {
                    if open.recording.lock().expect("the recording is intact").is_some() {
                        return Err(MobileError::Recording);
                    }
                    let directory = call.recordings.join("mobile");
                    tokio::fs::create_dir_all(&directory)
                        .await
                        .map_err(|error| MobileError::Io(format!("{}: {error}", directory.display())))?;
                    let file = directory.join(format!("{}-recording-{}.mp4", open.handle, millis()));
                    open.device.record_start(&file).await?;
                    *open.recording.lock().expect("the recording is intact") = Some((file, Instant::now()));
                    devices.recorded();
                    Outcome::result(
                        RecordResult {
                            device: open.handle.clone(),
                            recording: true,
                            path: None,
                            duration_ms: None,
                        },
                        format!("Recording {}; stop with demi mobile record {} stop.\n", open.handle, open.handle),
                    )
                }
                RecordAction::Stop => {
                    let Some((file, _)) = open.recording.lock().expect("the recording is intact").take() else {
                        return Err(MobileError::NotRecording);
                    };
                    devices.recorded();
                    let stopped = open.device.record_stop().await;
                    let recording = match stopped {
                        Ok(recording) => recording,
                        Err(error) => {
                            discard(&file).await;
                            return Err(error.into());
                        }
                    };
                    let seconds = recording.duration_ms as f64 / 1000.0;
                    let about = format!("{} · {} · {seconds:.1} s MP4", open.handle, open.name);
                    let output = match input.output {
                        Some(output) => Some((output, input.overwrite == Some(true))),
                        None => panel_file(call, &open, "mp4")?.map(|path| (path, false)),
                    };
                    match output {
                        Some((output, overwrite)) => {
                            let saved = save_file(call, &output, overwrite, &file).await;
                            discard(&file).await;
                            let path = saved?;
                            let text = format!("Saved {about} to {path}.\n");
                            Outcome::result(
                                RecordResult {
                                    device: open.handle.clone(),
                                    recording: false,
                                    path: Some(path),
                                    duration_ms: Some(recording.duration_ms),
                                },
                                text,
                            )
                        }
                        None => Ok(Outcome {
                            produced: Produced::File { file, about },
                            notes: Vec::new(),
                        }),
                    }
                }
            },
            (_, None) => unreachable!("an operation on a device found it open"),
        }
    };
    tokio::select! {
        biased;
        _ = call.cancel.cancelled() => Err(MobileError::Cancelled),
        done = done => done,
    }
}

/// Which of a device's commands an operation takes turns with
/// (`crates/command-package-mobile/src/live.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lane {
    /// It acts: the user's input waits for no read.
    Input,
    /// It reads or waits.
    Read,
    /// It ends the device: it waits for both.
    Both,
}

fn lane(operation: &MobileOperation) -> Lane {
    match operation {
        MobileOperation::Tap(_)
        | MobileOperation::Swipe(_)
        | MobileOperation::Type(_)
        | MobileOperation::Key(_)
        | MobileOperation::Rotate(_)
        | MobileOperation::Appearance(_) => Lane::Input,
        MobileOperation::Screenshot(_)
        | MobileOperation::Inspect(_)
        | MobileOperation::Wait(_)
        | MobileOperation::Record(_) => Lane::Read,
        MobileOperation::Close(_) | MobileOperation::List(_) | MobileOperation::Open(_) => Lane::Both,
    }
}

/// The operation's turn on `open`, held until dropped.
async fn turn(open: &OpenDevice, lane: Lane) -> Vec<tokio::sync::MutexGuard<'_, ()>> {
    match lane {
        Lane::Input => vec![open.inputs.lock().await],
        Lane::Read => vec![open.reads.lock().await],
        // Always the input first, so two that take both never hold one each.
        Lane::Both => vec![open.inputs.lock().await, open.reads.lock().await],
    }
}

async fn open_device(devices: &Arc<Devices>, call: &Call<'_>, query: &str) -> Result<Outcome, MobileError> {
    let open = devices.open(call.conversation, query).await?;
    if call.agent {
        // Opening is the agent's first command on the device, ended.
        drop(open.agent_uses());
    }
    let screen = *open.device.screen().borrow();
    let scale = Scale::of(&screen);
    let serial = open.device.serial();
    let native = match (open.platform, &serial) {
        (MobilePlatform::Ios, _) => format!("simctl id {}", open.native_id),
        (MobilePlatform::Android, Some(serial)) => format!("AVD {} · adb serial {serial}", open.native_id),
        (MobilePlatform::Android, None) => format!("AVD {}", open.native_id),
    };
    let text = format!(
        "{}  {} · {} · {}×{} · {native}\n",
        open.handle, open.name, open.os, scale.width, scale.height
    );
    Outcome::result(
        OpenResult {
            device: open.handle.clone(),
            name: open.name.clone(),
            platform: open.platform,
            os: open.os.clone(),
            native_id: open.native_id.clone(),
            serial,
            orientation: screen.orientation,
            width: scale.width,
            height: scale.height,
            booted: open.booted,
        },
        text,
    )
}

/// Waits for a settled picture: no new frame for [`QUIET`], at most
/// [`SETTLE_LIMIT`]. Answers whether it settled; a screen still changing
/// then, a video or a spinner, is taken as it is.
pub async fn settle(open: &OpenDevice) -> bool {
    let mut drawn = open.device.drawn();
    drawn.borrow_and_update();
    let limit = Instant::now() + SETTLE_LIMIT;
    loop {
        let quiet = Instant::now() + QUIET;
        tokio::select! {
            biased;
            _ = tokio::time::sleep_until(quiet) => return true,
            _ = tokio::time::sleep_until(limit) => return false,
            // A device that is gone draws nothing more.
            changed = drawn.changed() => if changed.is_err() { return true },
        }
    }
}

fn unsettled(call: &Call<'_>) -> String {
    format!(
        "{}: the screen was still changing after {:.1} s",
        call.command,
        SETTLE_LIMIT.as_secs_f64()
    )
}

/// Leaves out the last element, the deepest last one first, so the tree
/// fits; false when nothing is left to leave out.
fn shorten(elements: &mut Vec<demi_command_package_mobile_protocol::mobile::Element>) -> bool {
    let Some(last) = elements.last_mut() else {
        return false;
    };
    if shorten(&mut last.children) {
        return true;
    }
    elements.pop();
    true
}

/// The element `reference` names in the device's last inspect.
fn listed(open: &OpenDevice, reference: &str) -> Result<(ElementRef, Listed), MobileError> {
    let reference: ElementRef = reference
        .parse()
        .map_err(|error: String| MobileError::Usage(error))?;
    let refs = open.refs.lock().expect("the refs are intact");
    refs.get(&reference)
        .cloned()
        .map(|listed| (reference.clone(), listed))
        .ok_or(MobileError::StaleRef(reference))
}

fn summary(reference: ElementRef, listed: &Listed) -> demi_command_package_mobile_protocol::mobile::ElementSummary {
    demi_command_package_mobile_protocol::mobile::ElementSummary {
        r#ref: reference,
        role: listed.role.clone(),
        label: listed.label.clone(),
    }
}

/// The point under screenshot pixel `x`, `y`.
fn point(scale: &Scale, x: &str, y: &str) -> Result<Points, MobileError> {
    let number = |text: &str| {
        text.parse::<f64>()
            .ok()
            .filter(|value| value.is_finite())
            .ok_or_else(|| MobileError::Usage(format!("{text:?} is neither a number nor an element ref")))
    };
    let (x, y) = (number(x)?, number(y)?);
    scale.points(x, y).ok_or_else(|| {
        MobileError::Usage(format!(
            "{x},{y} is outside the screenshot, which is {}×{}",
            scale.width, scale.height
        ))
    })
}

/// A swipe across `listed` that moves its content `direction`: the finger
/// travels that way across the middle of its frame.
fn across(listed: &Listed, direction: Direction) -> (Points, Points) {
    let centre = listed.frame.centre();
    let (dx, dy) = match direction {
        Direction::Up => (0.0, -listed.frame.height),
        Direction::Down => (0.0, listed.frame.height),
        Direction::Left => (-listed.frame.width, 0.0),
        Direction::Right => (listed.frame.width, 0.0),
    };
    let half = SWIPE_SPAN / 2.0;
    (
        Points {
            x: centre.x - dx * half,
            y: centre.y - dy * half,
        },
        Points {
            x: centre.x + dx * half,
            y: centre.y + dy * half,
        },
    )
}

/// Delivers input at `at`, which the view marks when the agent taps
/// (`crates/command-package-mobile/src/live.rs`).
async fn acting<T>(
    open: &OpenDevice,
    call: &Call<'_>,
    at: Points,
    input: impl std::future::Future<Output = crate::driver::Result<T>>,
) -> Result<T, MobileError> {
    if call.agent {
        open.agent_tapped(LivePoint {
            x: at.x.clamp(0.0, 4096.0),
            y: at.y.clamp(0.0, 4096.0),
        });
    }
    Ok(input.await?)
}

async fn press(open: &OpenDevice, key: Key) -> Result<(), MobileError> {
    let button = |button| async move { Ok::<_, MobileError>(open.device.press(button).await?) };
    let usage = match key {
        Key::Home => return button(Button::Home).await,
        Key::Lock => return button(Button::Lock).await,
        Key::VolumeUp => return button(Button::VolumeUp).await,
        Key::VolumeDown => return button(Button::VolumeDown).await,
        Key::Back if open.platform == MobilePlatform::Android => return button(Button::Back).await,
        Key::Back => {
            return Err(MobileError::Usage(
                "back is Android's button; an iOS app goes back with its own control".into(),
            ));
        }
        Key::Enter => 0x28,
        Key::Escape => 0x29,
        Key::Delete => 0x2a,
        Key::Tab => 0x2b,
    };
    open.device.key(KeyPress { usage, down: true }).await?;
    Ok(open.device.key(KeyPress { usage, down: false }).await?)
}

async fn wait(
    open: &OpenDevice,
    input: demi_command_package_mobile_protocol::mobile::WaitInput,
) -> Result<Outcome, MobileError> {
    enum Until {
        Text(String),
        Element(ElementRef, Listed),
        Gone(ElementRef, Listed),
    }
    let until = match (input.text, input.element, input.gone) {
        (Some(text), None, None) => Until::Text(text),
        (None, Some(reference), None) => {
            let (reference, listed) = listed(open, reference.as_str())?;
            Until::Element(reference, listed)
        }
        (None, None, Some(reference)) => {
            let (reference, listed) = listed(open, reference.as_str())?;
            Until::Gone(reference, listed)
        }
        _ => {
            return Err(MobileError::Usage(
                "wait takes one of --text, --element and --gone".into(),
            ));
        }
    };
    let started = Instant::now();
    let seconds = input.timeout.unwrap_or(WAIT_SECONDS);
    let deadline = started + Duration::from_secs(seconds);
    let timed_out = || {
        let what = match &until {
            Until::Text(text) => format!("no element shows {}", text::quoted(text)),
            Until::Element(reference, _) => format!("{reference} did not appear"),
            Until::Gone(reference, _) => format!("{reference} is still there"),
        };
        MobileError::Timeout(format!("after {seconds} s {what}"))
    };
    loop {
        // A read under way at the deadline ends the wait too: a slow
        // screen's read takes seconds. The read dropped is safe to drop:
        // its answer, when it comes, finds nobody waiting and is let go.
        let Ok(raw) = tokio::time::timeout_at(deadline, open.device.tree()).await else {
            return Err(timed_out());
        };
        let raw = raw?;
        let found = match &until {
            Until::Text(text) => tree::find(&raw, &|element| {
                [&element.label, &element.value]
                    .into_iter()
                    .flatten()
                    .any(|shown| shown.contains(text.as_str()))
            })
            .map(|element| Some(element.clone())),
            Until::Element(_, listed) => tree::find(&raw, &|element| listed.matches(element))
                .map(|element| Some(element.clone())),
            Until::Gone(_, listed) => match tree::find(&raw, &|element| listed.matches(element)) {
                Some(_) => None,
                None => Some(None),
            },
        };
        if let Some(found) = found {
            let waited_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
            let element = match (&until, found) {
                (Until::Element(reference, listed), _) => Some(summary(reference.clone(), listed)),
                (Until::Text(_), Some(element)) => Some(demi_command_package_mobile_protocol::mobile::ElementSummary {
                    r#ref: {
                        let mut refs = open.refs.lock().expect("the refs are intact");
                        refs.add(Listed {
                            role: element.role.clone(),
                            label: element.label.clone(),
                            identifier: element.identifier.clone(),
                            frame: element.frame,
                        })
                    },
                    role: element.role,
                    label: element.label,
                }),
                _ => None,
            };
            let text = match (&until, &element) {
                (Until::Gone(reference, _), _) => format!("{reference} is gone.\n"),
                (_, Some(element)) => format!("Found {}.\n", text::summary(element)),
                (_, None) => "Found it.\n".to_owned(),
            };
            return Outcome::result(
                WaitResult {
                    device: open.handle.clone(),
                    element,
                    waited_ms,
                },
                text,
            );
        }
        if Instant::now() + WAIT_INTERVAL > deadline {
            return Err(timed_out());
        }
        tokio::time::sleep(WAIT_INTERVAL).await;
    }
}

/// A new file of the conversation's scratch directory for the user's
/// panel, `mobile/m1-<time>.<extension>`, which closing the device removes;
/// none for the agent's call, whose bytes go to standard output.
fn panel_file(call: &Call<'_>, open: &OpenDevice, extension: &str) -> Result<Option<String>, MobileError> {
    let Some(scratch) = &call.scratch else {
        return Ok(None);
    };
    let directory = scratch.join("mobile");
    std::fs::create_dir_all(&directory).map_err(|error| MobileError::Io(format!("{}: {error}", directory.display())))?;
    let path = directory.join(format!("{}-{}.{extension}", open.handle, millis()));
    open.saved.lock().expect("the saved files are intact").push(path.clone());
    Ok(Some(path.display().to_string()))
}

/// Milliseconds since the epoch, which name a saved file.
fn millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

/// Removes a recording's file nobody takes any more.
pub(crate) async fn discard(file: &std::path::Path) {
    if let Err(error) = tokio::fs::remove_file(file).await
        && error.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!("{} could not be removed: {error}", file.display());
    }
}

/// Saves the file `from` at `output`, resolved against the call's
/// directory, a piece at a time; answers the path.
async fn save_file(call: &Call<'_>, output: &str, overwrite: bool, from: &std::path::Path) -> Result<String, MobileError> {
    let path = demi_command_sdk::paths::resolve(call.cwd, output).map_err(|error| MobileError::Usage(error.to_string()))?;
    let shown = path.display().to_string();
    let mut file = tokio::fs::File::open(from)
        .await
        .map_err(|error| MobileError::Io(format!("{}: {error}", from.display())))?;
    match demi_shared_artifacts::publish(&path, &mut file, publication(overwrite), call.cancel).await {
        Ok(()) => Ok(shown),
        Err(demi_shared_artifacts::Error::Cancelled) => Err(MobileError::Cancelled),
        Err(demi_shared_artifacts::Error::Io(error)) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            Err(MobileError::OutputExists(shown))
        }
        Err(error) => Err(MobileError::Io(format!("{shown}: {error}"))),
    }
}

fn publication(overwrite: bool) -> demi_shared_artifacts::Publication {
    demi_shared_artifacts::Publication {
        mode: if overwrite {
            demi_shared_artifacts::Mode::Replace
        } else {
            demi_shared_artifacts::Mode::CreateNew
        },
        permissions: demi_shared_artifacts::Permissions::Default,
        durable: false,
    }
}

/// Saves `bytes` at `output`, resolved against `cwd`; answers the path.
async fn save(cwd: &str, output: &str, overwrite: bool, bytes: &[u8]) -> Result<String, MobileError> {
    let path = demi_command_sdk::paths::resolve(cwd, output).map_err(|error| MobileError::Usage(error.to_string()))?;
    let shown = path.display().to_string();
    match demi_shared_artifacts::publish_bytes(&path, bytes, publication(overwrite)).await {
        Ok(()) => Ok(shown),
        Err(demi_shared_artifacts::Error::Io(error)) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            Err(MobileError::OutputExists(shown))
        }
        Err(error) => Err(MobileError::Io(format!("{shown}: {error}"))),
    }
}

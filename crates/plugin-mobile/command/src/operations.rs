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
    InspectResult, Key, KeyResult, MobileOperation, OpenResult, Orientation,
    MobilePlatform, RecordAction, RecordInput, RecordTarget, Rotation, RotateResult,
    ScreenshotResult, SwipeResult, TapResult, TypeResult, WAIT_SECONDS, WaitResult,
};
use demi_live_view_protocol::LivePoint;
use demi_shared_recording::{Answer, Library, Reading, started};

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
    /// the user started; none for the agent's (`crates/plugin-mobile/command/src/live.rs`).
    pub scratch: Option<std::path::PathBuf>,
    /// The conversation's recordings on this Host, which go with the
    /// conversation whatever becomes of the program.
    pub library: Arc<Library>,
    /// The caller's own scratch directory, where an export of images goes
    /// unless told otherwise.
    pub caller_scratch: std::path::PathBuf,
    /// Whether the agent runs it, which the view shows; the user's panel
    /// controls are the user's, and show nothing (`crates/plugin-mobile/command/src/live.rs`).
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
    // A close needs no connection: a device being taken back closes at once.
    let open = match operation.device() {
        Some(handle) if !matches!(operation, MobileOperation::Close(_)) => {
            Some(devices.get(call.conversation, handle, call.cancel).await?)
        }
        _ => None,
    };
    // Commands on one device take turns in their lane. The agent uses it
    // from its start, the wait for its turn included.
    let turn_of = open.clone();
    let _agent = match &turn_of {
        Some(open) if call.agent => Some(open.agent_uses()),
        _ => None,
    };
    let _turn = match &turn_of {
        Some(open) => Some(turn(open, lane(&operation)).await),
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
            (MobileOperation::Close(input), _) => {
                let ended = devices.close(call.conversation, &input.device).await?;
                let text = match (&ended.failure, ended.shut_down) {
                    (Some(failure), _) => format!(
                        "Closed {}, but it did not shut down: {failure}. It stays booted; shut it down with the platform's own tools.\n",
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
                let elements = {
                    let mut refs = open.refs.lock().expect("the refs are intact");
                    tree::elements(&raw, input.keyboard == Some(true), &scale, &mut refs)
                };
                let result = InspectResult {
                    device: open.handle.clone(),
                    orientation: screen.orientation,
                    width: scale.width,
                    height: scale.height,
                    settled,
                    elements,
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
                // Turned from how the device is held, as Simulator.app turns
                // it, so a turn the app refused is passed by the next.
                let current = open.device.held();
                let next = match input.orientation {
                    Rotation::Portrait => Orientation::Portrait,
                    Rotation::Landscape if current.landscape() => current,
                    Rotation::Landscape => Orientation::LandscapeLeft,
                    Rotation::Left => current.left(),
                    Rotation::Right => current.right(),
                };
                open.device.orient(next).await?;
                // What the screen shows, which the app in front may have kept.
                let screen = *open.device.screen().borrow();
                let scale = Scale::of(&screen);
                let text = if screen.orientation == next {
                    format!(
                        "Turned {} {}; screenshots are {}×{}.\n",
                        open.handle,
                        text::orientation(next),
                        scale.width,
                        scale.height
                    )
                } else {
                    format!(
                        "{} stays {}: the app on screen does not turn {}; screenshots are {}×{}.\n",
                        open.handle,
                        text::orientation(screen.orientation),
                        text::orientation(next),
                        scale.width,
                        scale.height
                    )
                };
                Outcome::result(
                    RotateResult {
                        device: open.handle.clone(),
                        orientation: screen.orientation,
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
            (MobileOperation::Record(input), open) => record(call, open, input).await,
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
/// (`crates/plugin-mobile/command/src/live.rs`).
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
/// (`crates/plugin-mobile/command/src/live.rs`).
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
        tracing::warn!(event = "mobile_file.remove_failed", path = %file.display(), %error, "a file could not be removed");
    }
}

/// Saves the file `from` at `output`, resolved against the call's
/// directory, a piece at a time; answers the path.

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

/// `record`: starts recording a device, stops a recording, lists its frames,
/// or exports them.
async fn record(call: &Call<'_>, open: Option<Arc<OpenDevice>>, input: RecordInput) -> Result<Outcome, MobileError> {
    let output = match &input.output {
        Some(output) => Some(
            demi_command_sdk::paths::resolve(call.cwd, output).map_err(|error| MobileError::Usage(error.to_string()))?,
        ),
        None => None,
    };
    let reading = || reading_of(call, &input, output.as_deref());
    let answer = match (input.action, &input.target) {
        (RecordAction::Start, RecordTarget::Device(_)) => {
            let open = open.expect("an operation on a device found it open");
            if open.recording().is_some() {
                return Err(MobileError::Recording(open.handle.clone()));
            }
            let live = call.library.begin(&open.handle.to_string(), input.duration.map(|duration| duration.get())).await?;
            match open.device.record(live.sink()).await {
                Ok(capture) => {
                    // Only the panel's calls have a scratch directory, and its
                    // Stop takes the video of what it started.
                    if call.scratch.is_some() {
                        open.panel_started(live.id().clone());
                    }
                    open.record(live.clone(), capture);
                }
                Err(error) => {
                    call.library.discard(&live).await;
                    return Err(error.into());
                }
            }
            started(call.command, &live)?
        }
        (RecordAction::Start, RecordTarget::Recording(_)) => {
            return Err(MobileError::Usage(
                "start records a device, such as demi mobile record m1 start".into(),
            ));
        }
        (RecordAction::Stop, RecordTarget::Device(_)) => {
            let open = open.expect("an operation on a device found it open");
            let started = if call.scratch.is_some() { open.panel_takes() } else { None };
            let (id, stopped) = match open.recording() {
                Some(live) => (live.id().clone(), call.library.stop(live.id()).await?),
                // The recording the panel started ended by itself, at its
                // limit, or by the agent's stop: its video is the panel's all
                // the same.
                None => match started {
                    Some(id) => {
                        let stopped = call.library.stop(&id).await?;
                        (id, stopped)
                    }
                    _ => return Err(MobileError::NoRecording(format!("{} is not recording", open.handle))),
                },
            };
            // The panel's Stop saves the video, where the panel downloads it.
            match panel_file(call, &open, "mp4")? {
                Some(path) => {
                    let saved = std::path::PathBuf::from(&path);
                    let mut reading = reading();
                    reading.mp4 = true;
                    reading.output = Some(&saved);
                    reading.overwrite = true;
                    match call.library.export(&id, &reading).await {
                        Ok(answer) => answer,
                        // A recording that kept no frame has no video to save.
                        Err(demi_shared_recording::Error::BadRequest(_)) => stopped,
                        Err(error) => return Err(error.into()),
                    }
                }
                None => stopped,
            }
        }
        (RecordAction::Stop, RecordTarget::Recording(id)) => call.library.stop(id).await?,
        (RecordAction::Frames, RecordTarget::Recording(id)) => call.library.frames(id, &reading()).await?,
        (RecordAction::Export, RecordTarget::Recording(id)) => call.library.export(id, &reading()).await?,
        (_, RecordTarget::Device(_)) => {
            return Err(MobileError::Usage(format!(
                "{} works on a recording, such as demi mobile record r1 {}; start returns its ID",
                input.action, input.action
            )));
        }
    };
    match answer {
        Answer::Result { result, text } => Outcome::result(result, text),
        Answer::Video { file, about } => Ok(Outcome {
            produced: Produced::File { file, about },
            notes: Vec::new(),
        }),
    }
}

fn reading_of<'a>(call: &'a Call<'_>, input: &'a RecordInput, output: Option<&'a std::path::Path>) -> Reading<'a> {
    Reading {
        command: call.command,
        frames: input.frames.as_ref(),
        whole: input.whole == Some(true),
        region: input.region,
        mp4: input.mp4 == Some(true),
        output,
        overwrite: input.overwrite == Some(true),
        scratch: &call.caller_scratch,
    }
}

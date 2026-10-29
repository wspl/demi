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
    Appearance, AppearanceChoice, ElementSummary, AppearanceResult, CloseResult, Direction, ElementRef, EraseResult,
    FLAT_DEGREES, FoldResult, FoldTo, InstallResult, LaunchResult, Posture,
    InspectResult, Key, KeyResult, MobileOperation, OpenResult, Orientation,
    MobilePlatform, RecordAction, RecordInput, RecordTarget, Rotation, RotateResult,
    FIND_SECONDS, ScreenshotResult, SwipeResult, TapResult, TypeResult, TypedField, WAIT_SECONDS, WaitResult,
};
use demi_live_view_protocol::LivePoint;
use demi_shared_recording::{Answer, Library, Reading, started};
use demi_shared_recording_protocol::{Input, InputBy};

use crate::{
    devices::{Devices, OpenDevice},
    driver::{Button, Folded, KeyPress, Points},
    error::MobileError,
    refs::Listed,
    scale::Scale,
    target::{self, Selector, Target},
    text, tree, typing,
};

/// A settled picture: no new frame from the device for this long. Neither
/// platform's own idle signal is used, since both report idle while an app
/// draws its own animation; both drivers send a frame only when the screen
/// changes, so the live view's frames tell it at no cost. A screen whose
/// frames keep coming is settled too when two pictures of it this far
/// apart look the same ([`alike`]).
pub const QUIET: Duration = Duration::from_millis(300);
/// The share of pixels two pictures of a settled screen may differ in: a
/// spinner or a progress bar changes less, a page sliding or fading in
/// changes most of the screen. Maestro's threshold for a static screen.
const STILL_SHARE: f64 = 0.005;
/// How far a pixel's color may move, of 255, and the pixel still count as
/// the same: a simulator shades a still picture slightly differently from
/// one frame to the next.
const STILL_COLOR: u8 = 16;
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
    // Nor does a list of a device's recordings, which outlast it.
    let lists = matches!(&operation, MobileOperation::Record(input) if input.action == RecordAction::List);
    let open = match operation.device() {
        Some(handle) if !matches!(operation, MobileOperation::Close(_)) && !lists => {
            Some(devices.get(call.conversation, handle, call.cancel).await?)
        }
        _ => None,
    };
    // Commands on one device take turns in their queue. The agent uses it
    // from its start, the wait for its turn included.
    let turn_of = open.clone();
    let _agent = match &turn_of {
        Some(open) if call.agent => Some(open.agent_uses()),
        _ => None,
    };
    let _turn = match &turn_of {
        Some(open) => Some(turn(open, queue_of(&operation)).await),
        None => None,
    };
    // A recording of the device lists the agent's commands that act on it or
    // wait for it, with how each ended.
    let listed = match &turn_of {
        Some(open) if call.agent && LISTED.contains(&operation.name()) => {
            Some((
                open.command_begins(),
                demi_shared_recording_protocol::text::command(
                    operation.name(),
                    &operation.arguments(),
                    demi_command_package_mobile_protocol::mobile::positionals(operation.name()),
                ),
                operation.name(),
            ))
        }
        _ => None,
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
                let Settled { settled, picture } = settle(&open).await;
                let png = match picture {
                    Some(png) => png,
                    None => open.device.screenshot().await?,
                };
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
                let settled = settle(&open).await.settled;
                let raw = open.device.tree().await?;
                let screen = *open.device.screen().borrow();
                let scale = Scale::of(&screen);
                let elements = {
                    let mut refs = open.refs.lock().expect("the refs are intact");
                    refs.on(screen.fold.map(Folded::display));
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
                let text = text::inspect(&result, &open.name, screen.fold);
                let mut outcome = Outcome::result(result, text)?;
                if !settled {
                    outcome.notes.push(unsettled(call));
                }
                Ok(outcome)
            }
            (MobileOperation::Tap(input), Some(open)) => {
                let scale = Scale::of(&open.device.screen().borrow());
                let (at, element) = match input.at.as_slice() {
                    [target] => {
                        let listed = element(&open, target, input.timeout).await?;
                        (listed.1.frame.centre(), Some(listed))
                    }
                    [x, y] => (point(&scale, x, y)?, None),
                    _ => return Err(MobileError::Usage("tap takes an element, or x and y".into())),
                };
                let seconds = if input.long == Some(true) { LONG_PRESS_SECONDS } else { 0.0 };
                let _lift = open.touching(at, at);
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
                    [target, direction] => {
                        let (reference, listed) = element(&open, target, input.timeout).await?;
                        let direction: Direction = direction.parse().map_err(|_| {
                            MobileError::Usage(format!("{direction:?} is not up, down, left or right"))
                        })?;
                        let (from, to) = across(&listed, direction);
                        (from, to, Some(summary(reference, &listed)))
                    }
                    [x1, y1, x2, y2] => (point(&scale, x1, y1)?, point(&scale, x2, y2)?, None),
                    _ => {
                        return Err(MobileError::Usage(
                            "swipe takes x1 y1 x2 y2, or an element and up, down, left or right".into(),
                        ));
                    }
                };
                let _lift = open.touching(from, to);
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
                    Some(target) => Some(element(&open, target, input.timeout).await?),
                    None if replace => {
                        return Err(MobileError::Usage("--replace sets a field's text: name it with --into".into()));
                    }
                    None => None,
                };
                let characters = input.text.chars().count();
                let typed = match &target {
                    Some((_, listed)) if replace => {
                        let at = listed.frame.centre();
                        open.report_typed(&input.text, InputBy::Agent);
                        acting(&open, call, at, open.device.set_value(at, &input.text)).await?;
                        None
                    }
                    Some((_, listed)) => {
                        let at = listed.frame.centre();
                        {
                            let _lift = open.touching(at, at);
                            acting(&open, call, at, open.device.tap(at, 0.0)).await?;
                        }
                        Some(typed(&open, &input.text).await?)
                    }
                    None => Some(typed(&open, &input.text).await?),
                };
                let (fields, text) = match &typed {
                    Some(typed) => typed_text(&open, characters, typed),
                    None => {
                        let (reference, listed) = target.expect("--replace names its field");
                        let element = summary(reference, &listed);
                        let text = format!("Set {characters} characters into {}.\n", text::summary(&element));
                        (vec![TypedField { element, holds: None }], text)
                    }
                };
                Outcome::result(
                    TypeResult {
                        device: open.handle.clone(),
                        characters,
                        replaced: replace,
                        fields,
                        set: typed.is_some_and(|typed| typed.keyed.is_some()),
                    },
                    text,
                )
            }
            (MobileOperation::Key(input), Some(open)) => {
                press(&open, input.key, if call.agent { InputBy::Agent } else { InputBy::User }).await?;
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
            (MobileOperation::Fold(input), Some(open)) => {
                let Some(fold) = open.device.screen().borrow().fold else {
                    return Err(MobileError::Usage(format!("{} does not fold", open.handle)));
                };
                let to = FoldTo::parse(&input.to).ok_or_else(|| {
                    MobileError::Usage(format!("{:?} is not closed, half-open, open or an angle from 0 to 180", input.to))
                })?;
                let degrees = match (fold, to) {
                    (Folded::Hinge { .. }, to) => to.degrees(),
                    (Folded::Twofold { .. }, FoldTo::Posture(Posture::Closed)) => 0,
                    (Folded::Twofold { .. }, FoldTo::Posture(Posture::Open)) => FLAT_DEGREES,
                    (Folded::Twofold { .. }, _) => {
                        return Err(MobileError::Usage(format!("{} only folds closed or opens", open.handle)));
                    }
                };
                open.device.fold_to(degrees).await?;
                let screen = *open.device.screen().borrow();
                // A fold that shows the other display is done as a person
                // sees it once it shows: an app lays itself out anew for it,
                // and a touch sent before its first picture there may be lost
                // in the change. A hinge that moves on, as a slider drags it,
                // waits for nothing.
                let shown = screen.fold.map(Folded::display);
                let settled = shown == Some(fold.display()) || settle(&open).await.settled;
                let scale = Scale::of(&screen);
                let fold = screen.fold.unwrap_or(fold);
                let (posture, degrees, display) = (fold.posture(), fold.degrees(), fold.display());
                let text = format!(
                    "{} is {}: the {} display shows, {}×{}.\n",
                    open.handle,
                    text::folded(fold),
                    text::fold_display(display),
                    scale.width,
                    scale.height
                );
                let mut outcome = Outcome::result(
                    FoldResult {
                        device: open.handle.clone(),
                        posture,
                        degrees,
                        display,
                        width: scale.width,
                        height: scale.height,
                    },
                    text,
                )?;
                if !settled {
                    outcome.notes.push(unsettled(call));
                }
                Ok(outcome)
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
            (MobileOperation::Install(input), Some(open)) => {
                let file = std::path::Path::new(call.cwd).join(&input.file);
                if !file.exists() {
                    return Err(MobileError::Usage(format!("{} does not exist", file.display())));
                }
                let installed = open.device.install(&file).await?;
                let text = text::installed(&open.handle, &installed.app, installed.version.as_deref());
                Outcome::result(
                    InstallResult {
                        device: open.handle.clone(),
                        app: installed.app,
                        version: installed.version,
                    },
                    text,
                )
            }
            (MobileOperation::Launch(input), Some(open)) => {
                let launched = open.device.launch(&input.app, input.restart == Some(true)).await?;
                // Its first screen, as a person sees it once it shows.
                let settled = settle(&open).await.settled;
                let result = LaunchResult {
                    device: open.handle.clone(),
                    app: input.app,
                    started: launched.started,
                    console: launched.console,
                    shown: launched.shown,
                };
                let text = text::launched(&result);
                let mut outcome = Outcome::result(result, text)?;
                if !settled {
                    outcome.notes.push(unsettled(call));
                }
                Ok(outcome)
            }
            (MobileOperation::Logs(input), Some(open)) => {
                let app = match &input.app {
                    Some(app) => Some(open.device.app_entries(app).await?),
                    None => None,
                };
                let result = open.log.read(&open.handle, &input, app.as_ref())?;
                let text = text::logs(&result);
                Outcome::result(result, text)
            }
            (MobileOperation::Erase(_), Some(open)) => {
                devices.erase(&open).await?;
                Outcome::result(
                    EraseResult {
                        device: open.handle.clone(),
                    },
                    format!(
                        "Erased {} to its factory state and started it again; nothing of before is on it, the apps installed included.\n",
                        open.handle
                    ),
                )
            }
            (_, None) => unreachable!("an operation on a device found it open"),
        }
    };
    let outcome = tokio::select! {
        biased;
        _ = call.cancel.cancelled() => Err(MobileError::Cancelled),
        done = done => done,
    };
    if let Some((under_way, text, name)) = listed {
        let failed = outcome.as_ref().err().map(|error| {
            if demi_shared_recording_protocol::text::keeps_failure(name) {
                error.to_string()
            } else {
                "its message names the text it typed, which a recording does not keep".to_owned()
            }
        });
        under_way.ended(text, failed);
    }
    outcome
}

/// The commands a recording lists among its frames: those that act on the
/// device or wait for it, not those that only read it.
const LISTED: &[&str] = &["tap", "swipe", "type", "key", "rotate", "fold", "appearance", "launch", "install", "wait", "erase"];

/// Which of a device's commands an operation takes turns with
/// (`crates/plugin-mobile/command/src/live.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Queue {
    /// It acts: the user's input waits for no read.
    Input,
    /// It reads or waits.
    Read,
    /// It ends the device: it waits for both.
    Both,
    /// It reads what Demi keeps of the device, and waits for neither.
    Free,
}

fn queue_of(operation: &MobileOperation) -> Queue {
    match operation {
        MobileOperation::Tap(_)
        | MobileOperation::Swipe(_)
        | MobileOperation::Type(_)
        | MobileOperation::Key(_)
        | MobileOperation::Rotate(_)
        | MobileOperation::Fold(_)
        | MobileOperation::Appearance(_)
        | MobileOperation::Install(_)
        | MobileOperation::Launch(_) => Queue::Input,
        MobileOperation::Screenshot(_)
        | MobileOperation::Inspect(_)
        | MobileOperation::Wait(_)
        | MobileOperation::Record(_) => Queue::Read,
        MobileOperation::Close(_) | MobileOperation::List(_) | MobileOperation::Open(_) | MobileOperation::Erase(_) => {
            Queue::Both
        }
        MobileOperation::Logs(_) => Queue::Free,
    }
}

/// The operation's turn on `open`, held until dropped.
async fn turn(open: &OpenDevice, queue: Queue) -> Vec<tokio::sync::MutexGuard<'_, ()>> {
    match queue {
        Queue::Input => vec![open.inputs.lock().await],
        Queue::Read => vec![open.reads.lock().await],
        // Always the input first, so two that take both never hold one each.
        Queue::Both => vec![open.inputs.lock().await, open.reads.lock().await],
        Queue::Free => Vec::new(),
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
    let folded = screen.fold.map(|fold| format!(" · {}", text::folded(fold))).unwrap_or_default();
    let text = format!(
        "{}  {} · {} · {}×{}{folded} · {native}\n",
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
            posture: screen.fold.map(Folded::posture),
            booted: open.booted,
        },
        text,
    )
}

/// How a wait for a settled picture ended.
pub struct Settled {
    /// Whether the screen settled; a screen still changing at
    /// [`SETTLE_LIMIT`], a video or a page in transition, is taken as it is.
    pub settled: bool,
    /// The last of two pictures that looked the same, which is the settled
    /// screen; none when no frame came for [`QUIET`].
    pub picture: Option<crate::driver::Png>,
}

/// Waits for a settled picture: no new frame for [`QUIET`], or two
/// pictures [`QUIET`] apart that look the same, at most [`SETTLE_LIMIT`].
pub async fn settle(open: &OpenDevice) -> Settled {
    let limit = Instant::now() + SETTLE_LIMIT;
    tokio::select! {
        biased;
        () = quiet(open) => Settled { settled: true, picture: None },
        picture = looks_still(open) => Settled { settled: true, picture: Some(picture) },
        _ = tokio::time::sleep_until(limit) => Settled { settled: false, picture: None },
    }
}

/// Resolves once no new frame came for [`QUIET`].
async fn quiet(open: &OpenDevice) {
    let mut drawn = open.device.drawn();
    drawn.borrow_and_update();
    loop {
        tokio::select! {
            biased;
            _ = tokio::time::sleep(QUIET) => return,
            // A device that is gone draws nothing more.
            changed = drawn.changed() => if changed.is_err() { return },
        }
    }
}

/// Resolves with the second of two pictures [`QUIET`] apart that look the
/// same. It starts after the first [`QUIET`], which [`quiet`] tells a still
/// screen in at no cost; a picture that does not read leaves the wait to
/// [`quiet`].
async fn looks_still(open: &OpenDevice) -> crate::driver::Png {
    tokio::time::sleep(QUIET).await;
    let picture = || async {
        let png = open.device.screenshot().await.ok()?;
        let bytes = png.bytes.clone();
        let pixels = tokio::task::spawn_blocking(move || image::load_from_memory(&bytes).ok().map(|image| image.to_rgb8()))
            .await
            .ok()
            .flatten()?;
        Some((png, pixels))
    };
    // Pictures are QUIET apart from when each is taken: a picture itself
    // takes a few hundred milliseconds on iOS.
    let mut taken = Instant::now();
    let Some((_, mut before)) = picture().await else {
        return std::future::pending().await;
    };
    loop {
        tokio::time::sleep_until(taken + QUIET).await;
        taken = Instant::now();
        let Some((png, after)) = picture().await else {
            return std::future::pending().await;
        };
        let (still, after) = tokio::task::spawn_blocking(move || (alike(&before, &after), after))
            .await
            .unwrap_or((false, image::RgbImage::new(0, 0)));
        if still {
            return png;
        }
        before = after;
    }
}

/// Whether two pictures of the screen look the same: at most
/// [`STILL_SHARE`] of their pixels differ by more than [`STILL_COLOR`].
fn alike(before: &image::RgbImage, after: &image::RgbImage) -> bool {
    if before.dimensions() != after.dimensions() || before.is_empty() {
        return false;
    }
    let differ = before
        .pixels()
        .zip(after.pixels())
        .filter(|(a, b)| a.0.iter().zip(b.0).any(|(a, b)| a.abs_diff(b) > STILL_COLOR))
        .count();
    (differ as f64) <= STILL_SHARE * f64::from(before.width()) * f64::from(before.height())
}

fn unsettled(call: &Call<'_>) -> String {
    format!(
        "{}: the screen was still changing after {:.1} s",
        call.command,
        SETTLE_LIMIT.as_secs_f64()
    )
}

/// The element `target` names, with its ref: a ref's, as the last inspect
/// listed it, or the one a `text=` or `id=` finds once it is there, waiting
/// at most `timeout` seconds.
async fn element(open: &OpenDevice, target: &str, timeout: Option<u64>) -> Result<(ElementRef, Listed), MobileError> {
    match Target::parse(target)? {
        Target::Ref(reference) => listed(open, reference),
        Target::Find(selector) => {
            target::find(open, &selector, Duration::from_secs(timeout.unwrap_or(FIND_SECONDS))).await
        }
    }
}

/// The element `reference` names in the device's last inspect.
fn listed(open: &OpenDevice, reference: ElementRef) -> Result<(ElementRef, Listed), MobileError> {
    let display = open.device.screen().borrow().fold.map(Folded::display);
    let refs = open.refs.lock().expect("the refs are intact");
    match refs.get(&reference, display) {
        Some(listed) => Ok((reference, listed.clone())),
        None => Err(MobileError::StaleRef(reference)),
    }
}

fn summary(reference: ElementRef, listed: &Listed) -> demi_command_package_mobile_protocol::mobile::ElementSummary {
    demi_command_package_mobile_protocol::mobile::ElementSummary {
        r#ref: reference,
        role: listed.role.clone(),
        label: listed.label.clone(),
    }
}

/// The fields `typed` went into, each with a reference, and what `type`
/// says of them.
fn typed_text(open: &OpenDevice, characters: usize, typed: &typing::Typed) -> (Vec<TypedField>, String) {
    let fields: Vec<TypedField> = {
        let display = open.device.screen().borrow().fold.map(Folded::display);
        let mut refs = open.refs.lock().expect("the refs are intact");
        refs.on(display);
        typed
            .fields
            .iter()
            .map(|field| {
                let listed = Listed {
                    role: field.role.clone(),
                    label: field.label.clone(),
                    identifier: field.identifier.clone(),
                    frame: field.frame,
                };
                TypedField {
                    element: summary(refs.add(listed.clone()), &listed),
                    holds: (typed.checked && !field.secure).then(|| field.text.clone()),
                }
            })
            .collect()
    };
    let mut text = match fields.as_slice() {
        [] => format!(
            "Typed {characters} characters; the screen's tree names no focused field, so they went to whatever takes the keys, unchecked: a screenshot shows where they went"
        ),
        [field] => format!("Typed {characters} characters into {}", text::summary(&field.element)),
        fields => {
            let held: Vec<String> = fields
                .iter()
                .map(|field| text::quoted(field.holds.as_deref().unwrap_or_default()))
                .collect();
            format!("Typed {characters} characters into {} fields: {}", fields.len(), held.join(", "))
        }
    };
    if let Some(keyed) = &typed.keyed {
        text.push_str(&format!("; the keys left {}, so it set the text directly", text::quoted(keyed)));
    }
    if let [field] = fields.as_slice()
        && let Some(holds) = &field.holds
    {
        text.push_str(&format!("; it holds {}", text::quoted(holds)));
    }
    if typed.gone {
        text.push_str("; the field is gone since, as the app acted on the text");
    }
    if typed.fields.iter().any(|field| field.secure) {
        text.push_str("; a password field's text cannot be read, so it was not checked");
    }
    if let Some(acted) = typed.acted {
        let key = if acted == '\n' { "Enter" } else { "Tab" };
        text.push_str(&format!("; {key} and what follows it went in unchecked, as they act on the app"));
    }
    text.push_str(".\n");
    (fields, text)
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

async fn press(open: &OpenDevice, key: Key, by: InputBy) -> Result<(), MobileError> {
    open.report(by, |scale| match key {
        Key::Enter | Key::Escape | Key::Delete | Key::Tab => Input::Keys {
            keys: vec![key.to_string()],
            scale,
            by,
        },
        _ => Input::Button {
            name: key.to_string(),
            scale,
            by,
        },
    });
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
    let started = Instant::now();
    let seconds = input.timeout.unwrap_or(WAIT_SECONDS);
    let timeout = Duration::from_secs(seconds);
    let waited = || u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let found = |element: ElementSummary| {
        let text = format!("Found {}.\n", text::summary(&element));
        Outcome::result(
            WaitResult {
                device: open.handle.clone(),
                element: Some(element),
                waited_ms: waited(),
            },
            text,
        )
    };
    /// What `wait` reads the tree for until it holds.
    enum Until {
        /// Any element shows the text.
        Shows(Selector),
        /// The element a ref names is there, or is gone.
        Listed { reference: ElementRef, listed: Listed, gone: bool },
        /// No element matches.
        NoneMatches(Selector),
    }
    let until = match (input.text, input.element, input.gone) {
        (Some(text), None, None) => Until::Shows(Selector::Text(text)),
        (None, Some(target), None) => match Target::parse(&target)? {
            Target::Ref(reference) => {
                let (reference, listed) = listed(open, reference)?;
                Until::Listed { reference, listed, gone: false }
            }
            // As tap finds it: the only match, on the screen, enabled and
            // settled.
            Target::Find(selector) => {
                let (reference, listed) = target::find(open, &selector, timeout).await?;
                return found(summary(reference, &listed));
            }
        },
        (None, None, Some(target)) => match Target::parse(&target)? {
            Target::Ref(reference) => {
                let (reference, listed) = listed(open, reference)?;
                Until::Listed { reference, listed, gone: true }
            }
            Target::Find(selector) => Until::NoneMatches(selector),
        },
        _ => {
            return Err(MobileError::Usage(
                "wait takes one of --text, --element and --gone".into(),
            ));
        }
    };
    let deadline = started + timeout;
    let timed_out = || {
        let what = match &until {
            Until::Shows(selector) => format!("no element {}", selector.said(true)),
            Until::Listed { reference, gone: false, .. } => format!("{reference} did not appear"),
            Until::Listed { reference, gone: true, .. } => format!("{reference} is still there"),
            Until::NoneMatches(selector) => format!("an element that {} is still there", selector.said(true)),
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
        match &until {
            Until::Shows(selector) => {
                if let Some(element) = selector.matches(&raw).first() {
                    let (reference, listed) = target::referenced(open, element);
                    return found(summary(reference, &listed));
                }
            }
            Until::Listed { reference, listed, gone } => {
                let there = tree::find(&raw, &|element| listed.matches(element)).is_some();
                match (there, gone) {
                    (true, false) => return found(summary(reference.clone(), listed)),
                    (false, true) => {
                        return Outcome::result(
                            WaitResult {
                                device: open.handle.clone(),
                                element: None,
                                waited_ms: waited(),
                            },
                            format!("{reference} is gone.\n"),
                        );
                    }
                    _ => {}
                }
            }
            Until::NoneMatches(selector) => {
                if selector.matches(&raw).is_empty() {
                    return Outcome::result(
                        WaitResult {
                            device: open.handle.clone(),
                            element: None,
                            waited_ms: waited(),
                        },
                        format!("No element {} now.\n", selector.said(true)),
                    );
                }
            }
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
    if let Some(why) = input.misfit() {
        return Err(MobileError::Usage(why));
    }
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
        (RecordAction::Start, RecordTarget::Recording(_) | RecordTarget::Recordings(_)) => {
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
                    match call.library.export(std::slice::from_ref(&id), &reading).await {
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
        (RecordAction::Export, RecordTarget::Recording(_) | RecordTarget::Recordings(_)) => {
            call.library.export(&input.target.recordings(), &reading()).await?
        }
        (RecordAction::Note, RecordTarget::Recording(id)) => {
            let at = input.at.map(std::time::Duration::from_secs_f64);
            call.library.note(id, input.text.as_deref().unwrap_or_default(), at).await?
        }
        // Several recordings are for an export alone, which `misfit` said.
        (RecordAction::Stop | RecordAction::Frames | RecordAction::Note, RecordTarget::Recordings(_)) => {
            unreachable!("several recordings are only exported")
        }
        (RecordAction::List, RecordTarget::Device(device)) => call.library.listing(&device.to_string()).await?,
        (RecordAction::List, RecordTarget::Recording(_) | RecordTarget::Recordings(_)) => {
            return Err(MobileError::Usage("list names a device, such as demi mobile record m1 list".into()));
        }
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
        plain: input.plain == Some(true),
        output,
        overwrite: input.overwrite == Some(true),
        fast_waits: input.fast_waits == Some(true),
        sheet: input.sheet == Some(true),
        every: input.every.map(|every| every.get()),
        scratch: &call.caller_scratch,
    }
}

/// Types `text` on the device as the agent, the recording drawing each run
/// of keys as it goes out.
async fn typed(open: &Arc<OpenDevice>, text: &str) -> Result<typing::Typed, MobileError> {
    let device = open.clone();
    let told: Arc<dyn Fn(&str) + Send + Sync> = Arc::new(move |run: &str| device.report_typed(run, InputBy::Agent));
    crate::keys::TYPED.scope(told, typing::type_text(&open.device, text)).await
}

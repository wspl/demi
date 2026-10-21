//! A tab's recording: the capture extension codes every frame the tab draws
//! (`driver/capture/offscreen.js`), and this task hands the pictures to the
//! recording's sink (`demi-shared-recording`).
//!
//! ```text
//! tab --capture--> extension: encodes every frame --pictures--> this task
//!                                                                  |
//!                  resize, stop, tab closed, duration              v
//!                  --> the extension hands over what it       Sink (files)
//!                      holds first
//! ```
//!
//! # Rules
//!
//! - The recording takes the tab's capture where the live view has opened
//!   one, and opens one otherwise: Chrome grants a tab one capture. Either
//!   way it is at the tab's own pixels, which the live view's stream scales
//!   its pictures from, so a viewer's window or a slow network never changes
//!   what a recording keeps.
//! - A resize of the tab waits for every capture of it to be released
//!   (`CaptureGate`), so the recording ends its capture for the resize and
//!   starts again once the tab has its new size: the video changes size
//!   there, at a key frame.
//! - An end hands over first: the extension codes the frames it holds, and
//!   the pictures are delivered before the recording is told to end, so the
//!   last frames before a stop are kept.
//! - A failed capture is started again, as the live view's is, until it fails
//!   three times in a row; the recording then ends failed and says why.

use std::time::Duration;

use tokio::time::Instant;
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use demi_shared_recording::{Handover, Live, Picture, Sink};
use demi_shared_recording_protocol::{Area, EndReason};

use crate::driver::capture::{CaptureChannel, RecordEvent};
use crate::driver::operation::BrowserError;
use crate::tabs::{tab::BrowserTab, viewport::pixels};

/// How long a capture that failed waits before it starts again.
const RETRY: Duration = Duration::from_secs(1);
/// How many captures in a row may fail before the recording does.
const FAILURES: u32 = 3;
/// A capture that ran this long was well, whatever failed it after.
const WELL: Duration = Duration::from_secs(10);
/// How long the pictures a capture holds may take to arrive once it was
/// asked to end.
const FINISH: Duration = Duration::from_secs(5);

/// Starts recording `tab` into `live`. The task ends with the recording, and
/// ends the recording when the tab closes or the duration runs out.
pub fn start(tab: BrowserTab, captures: CaptureChannel, live: Live, tasks: &TaskTracker) {
    tasks.spawn(async move {
        let sink = live.sink();
        let handover = live.handover().clone();
        // Cancelled when the tab closes or the limit comes: the session hands
        // over what the extension holds and ends.
        let over = CancellationToken::new();
        let session = session(&tab, &captures, &sink, &over, &handover);
        tokio::pin!(session);
        let end = tokio::select! {
            end = &mut session => end,
            Some(why) = live.over(tab.ended()) => {
                over.cancel();
                let _late = tokio::time::timeout(FINISH * 2, &mut session).await;
                Ending::Over(why)
            }
            _ = sink.ended().cancelled() => return,
        };
        match end {
            // A stop asked for the handover, and ends the recording itself.
            Ending::Handed => {}
            // A tab that closed took its capture with it.
            Ending::Failed(_) if tab.ended().is_cancelled() => sink.end(EndReason::Closed, None),
            Ending::Failed(note) => sink.end(EndReason::Failed, Some(note)),
            Ending::Over(reason) => sink.end(reason, None),
        }
        // Whoever stops the recording next has nothing to wait for.
        handover.done();
        // The task is the recording's until its files are complete.
        sink.ended().cancelled().await;
    });
}

/// Why the recording's session ended.
enum Ending {
    /// A stop asked for the handover, and it is done.
    Handed,
    Failed(String),
    /// The tab closed or the limit came, and the handover is done: the
    /// recording ends for this reason.
    Over(EndReason),
}

/// Runs the recording's captures in turn until an end asks for the handover,
/// or a capture fails past retrying.
async fn session(
    tab: &BrowserTab,
    captures: &CaptureChannel,
    sink: &Sink,
    over: &CancellationToken,
    handover: &Handover,
) -> Ending {
    let mut failures = 0;
    loop {
        let started = Instant::now();
        match capture(tab, captures, sink, over, handover).await {
            Capture::Handed => return Ending::Handed,
            // The tab resized; the next capture starts at its new size.
            Capture::Resized => failures = 0,
            Capture::Failed(note) => {
                if started.elapsed() >= WELL {
                    failures = 0;
                }
                failures += 1;
                if failures >= FAILURES {
                    return Ending::Failed(note);
                }
                tracing::warn!("a recording's capture failed, starting again: {note}");
                tokio::select! {
                    _ = over.cancelled() => return Ending::Handed,
                    _ = handover.requested().cancelled() => return Ending::Handed,
                    _ = tokio::time::sleep(RETRY) => {}
                }
            }
        }
    }
}

/// How one capture ended.
enum Capture {
    /// A stop asked the extension to hand over, and it did.
    Handed,
    /// The tab is being resized, which the capture made way for.
    Resized,
    Failed(String),
}

/// One capture of the tab: from its start until a stop, a resize or a
/// failure.
async fn capture(
    tab: &BrowserTab,
    captures: &CaptureChannel,
    sink: &Sink,
    over: &CancellationToken,
    handover: &Handover,
) -> Capture {
    let mut gate = tab.state().capture.subscribe();
    // A resize that has begun keeps the capture off until it ends.
    if gate.wait_for(|gated| !gated.resizing()).await.is_err() {
        return Capture::Failed("the tab is gone".into());
    }
    let Some(hold) = tab.state().capture.capture() else {
        return Capture::Resized;
    };
    let size = {
        let viewports = tab.state().viewport.borrow();
        pixels(&viewports.current, 1.0)
    };
    let never = CancellationToken::new();
    let mut recording = match captures.record(tab.target_id(), size.0, size.1, hold, &never).await {
        Ok(recording) => recording,
        Err(BrowserError::Cancelled) => return Capture::Failed("the recording was cancelled".into()),
        Err(error) => return Capture::Failed(error.to_string()),
    };
    // Why the capture is being finished, once it is.
    let mut finishing: Option<(Capture, Instant)> = None;
    loop {
        let waiting = finishing.as_ref().map(|(_, since)| *since + FINISH);
        tokio::select! {
            event = recording.events.recv() => match event {
                Some(RecordEvent::Picture(frame, changed)) => {
                    let taken = sink.push_picture(Picture {
                        data: frame.data.to_vec(),
                        key: frame.key,
                        at_us: frame.timestamp,
                        width: u32::from(frame.width),
                        height: u32::from(frame.height),
                        changed: Area {
                            x: u32::from(changed.x),
                            y: u32::from(changed.y),
                            width: u32::from(changed.width),
                            height: u32::from(changed.height),
                        },
                    });
                    if !taken {
                        // The recording ended, or failed.
                        return Capture::Failed("the recording no longer takes pictures".into());
                    }
                }
                Some(RecordEvent::Missed(count)) => sink.miss(count),
                // A tab that draws nothing new sends no frame for a capture that
                // starts, so it draws itself once.
                Some(RecordEvent::Started | RecordEvent::Stalled) => redraw(tab),
                Some(RecordEvent::Failed(note)) => return Capture::Failed(note),
                Some(RecordEvent::Stopped) => {
                    return finishing.map_or(Capture::Failed("the capture ended".into()), |(why, _)| why);
                }
                None => return Capture::Failed("the capture ended".into()),
            },
            _ = async { tokio::select! {
                _ = handover.requested().cancelled() => {}
                _ = over.cancelled() => {}
            } }, if finishing.is_none() => {
                recording.finish().await;
                finishing = Some((Capture::Handed, Instant::now()));
            }
            // Whenever a resize begins, not only when one had when this began
            // waiting: it waits for this capture to be released.
            _ = async { gate.wait_for(|gated| gated.resizing()).await.is_ok() }, if finishing.is_none() => {
                recording.finish().await;
                finishing = Some((Capture::Resized, Instant::now()));
            }
            _ = async { tokio::time::sleep_until(waiting.expect("it is waiting")).await }, if waiting.is_some() => {
                return finishing.map_or(Capture::Failed("the capture ended".into()), |(why, _)| why);
            }
        }
    }
}

/// Paints the tab once, as the live view does: a still page draws nothing
/// new for a capture that starts.
fn redraw(tab: &BrowserTab) {
    let page = tab.page().clone();
    let ended = tab.ended().clone();
    tokio::spawn(async move {
        tokio::select! {
            _ = ended.cancelled() => {}
            _painted = tokio::time::timeout(Duration::from_secs(5), crate::tabs::viewport::paint(&page)) => {}
        }
    });
}

//! A recording's capture of an emulator: every picture its screen draws that
//! differs from the one before, coded as H.264 at the screen's own pixels,
//! from the emulator's one stream of pictures (`screen.rs`).
//!
//! ```text
//! screen --picture--> queue --> thread: same as the last kept? drop it
//!                                       else encode, say where it differs
//!                                              |
//!                                              v
//!                                        Sink::push_picture
//! ```
//!
//! The thread codes behind the stream, so a slow encoder never slows the
//! emulator's reader: a picture that finds the queue full is not lost to the
//! screen but to the recording, and is counted as missed, as are the pictures
//! the emulator says, by its sequence numbers, it dropped for a slow reader.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
    mpsc,
};

use tokio_util::task::AbortOnDropHandle;

use demi_shared_h264::{Encoder, PixelFormat, Raw, Rate, Settings};
use demi_shared_openh264::Load;
use demi_shared_recording::{Held, KEY_INTERVAL, Picture, Settled, Sink, changed};
use demi_shared_recording_protocol::{Area, EndReason};

use super::screen::{self, Event, Screen, Subscription};

/// How many pictures may wait for the encoder before one is counted missed:
/// pictures are large, so a slow encoder must not hold them without end.
const QUEUE: usize = 8;

/// What the thread is told.
enum Message {
    Picture(Arc<screen::Picture>, u64),
    Missed(u32),
    /// The pictures held are coded and delivered; the emulator's stream ended
    /// when `ended` is a reason.
    Finish { ended: Option<String> },
}

/// A recording's capture: dropping it stops it.
pub struct Capture {
    _subscription: Subscription,
    _handover: AbortOnDropHandle<()>,
}

/// Starts handing `sink` the emulator's pictures. The recording ends, closed,
/// once the emulator stops sending.
pub fn start(screen: &Arc<Screen>, load: Load, sink: Sink) -> Capture {
    let (messages, inbox) = mpsc::channel();
    let waiting = Arc::new(AtomicUsize::new(0));
    let thread_sink = sink.clone();
    let thread_waiting = waiting.clone();
    std::thread::Builder::new()
        .name("demi-mobile-record".into())
        .spawn(move || run(&inbox, &thread_waiting, &load, &thread_sink))
        .expect("the recording's thread starts");
    let sent = messages.clone();
    let need = screen.recording();
    let subscription = screen.subscribe(need, move |event| {
        let message = match event {
            // The latest picture a live view alone had the stream draw is
            // smaller than the recording's, and is not its first frame.
            Event::Picture(picture, _) if picture.width.max(picture.height) < need => return,
            Event::Picture(picture, at) => {
                if waiting.fetch_add(1, Ordering::AcqRel) >= QUEUE {
                    waiting.fetch_sub(1, Ordering::AcqRel);
                    Message::Missed(1)
                } else {
                    Message::Picture(picture, at)
                }
            }
            Event::Missed(count) => Message::Missed(count),
            Event::Ended(note) => Message::Finish { ended: Some(note) },
        };
        let _gone = sent.send(message);
    });
    // A stop asks for what the encoder holds before the recording ends.
    let handover = sink.handover().clone();
    let asking = AbortOnDropHandle::new(tokio::spawn(async move {
        handover.requested().cancelled().await;
        let _gone = messages.send(Message::Finish { ended: None });
    }));
    Capture {
        _subscription: subscription,
        _handover: asking,
    }
}

/// What the recording thread has kept and holds.
struct Run<'a> {
    load: &'a Load,
    sink: &'a Sink,
    encoder: Option<(Box<dyn Encoder>, (u32, u32))>,
    /// The picture given to the encoder last, which the next is compared with.
    last: Option<Arc<screen::Picture>>,
    /// Where each picture the encoder holds differs, until it gives the
    /// picture back or says it dropped it.
    held: Held,
    /// Pictures given to the encoder since its last key frame.
    since_key: u32,
}

fn run(inbox: &mpsc::Receiver<Message>, waiting: &AtomicUsize, load: &Load, sink: &Sink) {
    let mut run = Run {
        load,
        sink,
        encoder: None,
        last: None,
        held: Held::default(),
        since_key: 0,
    };
    let ended = loop {
        match inbox.recv() {
            Ok(Message::Missed(count)) => sink.miss(count),
            Ok(Message::Picture(picture, at)) => {
                waiting.fetch_sub(1, Ordering::AcqRel);
                if let Err(error) = run.keep(&picture, at) {
                    sink.end(EndReason::Failed, Some(error));
                    break None;
                }
            }
            Ok(Message::Finish { ended }) => break ended,
            // The capture was dropped.
            Err(_) => break None,
        }
    };
    if let Err(error) = run.flush() {
        sink.end(EndReason::Failed, Some(error));
    }
    if let Some(note) = ended {
        sink.end(EndReason::Closed, Some(note));
    }
    sink.handover().done();
}

impl Run<'_> {
    /// Codes `picture`, drawn at `at`, unless it is the one kept last.
    fn keep(&mut self, picture: &Arc<screen::Picture>, at: u64) -> Result<(), String> {
        let (width, height) = (picture.width & !1, picture.height & !1);
        let pixels = evened(picture, width, height);
        let now = raw(&pixels, width, height);
        let area = match &self.last {
            Some(before) if (before.width & !1, before.height & !1) == (width, height) => {
                let before = evened(before, width, height);
                match changed(&now, &raw(&before, width, height)) {
                    Some(area) => area,
                    None => return Ok(()),
                }
            }
            _ => Area::whole(width, height),
        };
        let restart = self.encoder.as_ref().is_none_or(|(_, size)| *size != (width, height));
        if restart {
            // The video changes size at a key frame; what the old encoder
            // holds is delivered first.
            self.flush()?;
            let settings = Settings {
                width,
                height,
                rate: Rate::Quality,
            };
            let encoder = demi_shared_h264::open(settings, self.load).map_err(|error| format!("the video encoder did not start: {error}"))?;
            tracing::debug!(event = "android_encoder.chosen", purpose = "record", encoder = encoder.name(), "an encoder was chosen");
            self.encoder = Some((encoder, (width, height)));
            self.since_key = 0;
        }
        let key = restart || self.since_key >= KEY_INTERVAL;
        let (encoder, _) = self.encoder.as_mut().expect("an encoder is started");
        self.held.give(at, area);
        let coded = encoder.encode(now, at, key).map_err(|error| format!("a picture did not encode: {error}"))?;
        let dropped = encoder.take_dropped();
        self.since_key = if key { 1 } else { self.since_key + 1 };
        self.last = Some(picture.clone());
        self.deliver(coded, &dropped, (width, height));
        Ok(())
    }

    /// Delivers the pictures the encoder still holds.
    fn flush(&mut self) -> Result<(), String> {
        let Some((encoder, size)) = &mut self.encoder else {
            return Ok(());
        };
        let size = *size;
        let coded = encoder.finish().map_err(|error| format!("the pictures held did not encode: {error}"))?;
        let dropped = encoder.take_dropped();
        self.encoder = None;
        self.deliver(coded, &dropped, size);
        self.held.clear();
        Ok(())
    }

    /// Hands the sink what the encoder gave and counts what it dropped.
    fn deliver(&mut self, mut coded: Vec<demi_shared_h264::Picture>, dropped: &[u64], (width, height): (u32, u32)) {
        let given: Vec<u64> = coded.iter().map(|picture| picture.time_us).collect();
        for settled in self.held.settle(&given, dropped) {
            match settled {
                Settled::Dropped => self.sink.miss(1),
                Settled::Given { at, changed } => {
                    let Some(place) = coded.iter().position(|picture| picture.time_us == at) else {
                        continue;
                    };
                    let picture = coded.remove(place);
                    self.sink.push_picture(Picture {
                        data: picture.data,
                        key: picture.key,
                        at_us: picture.time_us as f64,
                        width,
                        height,
                        changed,
                    });
                }
            }
        }
        // A picture the encoder gave that no draw is held for is a defect of
        // the encoder; its place is not known, so it is the whole picture.
        for picture in coded {
            tracing::warn!(event = "android_record.picture_unmatched", time_us = picture.time_us, "a recording's encoder gave a picture nobody drew");
            self.sink.push_picture(Picture {
                data: picture.data,
                key: picture.key,
                at_us: picture.time_us as f64,
                width,
                height,
                changed: Area::whole(width, height),
            });
        }
    }
}

/// `picture`'s pixels cut to `width` × `height`, its own when it is that
/// size already; H.264 has no odd side.
fn evened(picture: &screen::Picture, width: u32, height: u32) -> std::borrow::Cow<'_, [u8]> {
    if (picture.width, picture.height) == (width, height) {
        return std::borrow::Cow::Borrowed(&picture.rgba);
    }
    let row = picture.width as usize * 4;
    let mut cut = Vec::with_capacity(width as usize * height as usize * 4);
    for rows in picture.rgba.chunks_exact(row).take(height as usize) {
        cut.extend_from_slice(&rows[..width as usize * 4]);
    }
    std::borrow::Cow::Owned(cut)
}

fn raw(pixels: &[u8], width: u32, height: u32) -> Raw<'_> {
    Raw {
        width,
        height,
        format: PixelFormat::Rgba,
        pixels,
    }
}

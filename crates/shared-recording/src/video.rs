//! The video of one recording or several, one after another, as an MP4.
//!
//! A plain video of one recording of one size is its own pictures, each
//! lasting until the next was drawn. Any other is decoded, drawn on and
//! coded again: the inputs on the frames (`overlay.rs`), and the notes and
//! the badge of a wait played faster (`caption.rs`), at the recordings' own
//! frames and, while something drawn moves, 30 times a second between them.
//! Videos of several recordings are coded again even plain: two coders'
//! parameter sets differ, and a player keeps the first.
//!
//! A video coded again has one size, its first picture's, and a picture of
//! another size is scaled to fit it, in its middle on black, as a video
//! editor fits a clip to its project: an MP4 whose pictures change size names
//! the first size as the whole track's, which players stretch the others to,
//! and a player keeps the parameter sets of the first sample entry, which
//! cannot decode another.

use std::{
    fs,
    io::{Read as _, Seek as _, SeekFrom},
    path::{Path, PathBuf},
};

use demi_shared_h264::Encoder as _;
use demi_shared_openh264::Load;
use demi_shared_recording_protocol::{Input, RecordingId, frame_label, text::seconds};
use openh264::decoder::{Decoder, DecoderConfig};

use crate::{
    Error,
    caption::{self, Caption},
    export::decodable,
    library::Stored,
    mp4,
    overlay::Overlay,
    pace::Pace,
    stored::{Contents, VIDEO},
};

/// One recording of a video.
pub(crate) struct Part {
    pub id: RecordingId,
    directory: PathBuf,
    pub contents: Contents,
    /// How long it ran, in microseconds.
    pub ran: u64,
    /// Its notes, each at its time.
    notes: Vec<(u64, String)>,
}

impl Part {
    pub(crate) fn of(stored: &Stored) -> Result<Self, Error> {
        let contents = stored.contents()?;
        if contents.frames.is_empty() {
            return Err(Error::BadRequest(format!("{} kept no frames", stored.id)));
        }
        let ran = match (&contents.meta.ended, &stored.live) {
            (Some(ended), _) => ended.duration_us,
            (None, Some(live)) => live.info()?.duration_ms * 1000,
            (None, None) => contents.frames.last().map_or(0, |last| last.t),
        };
        let notes = contents
            .inputs
            .iter()
            .filter_map(|row| match &row.input {
                Input::Note { text } => Some((row.time_us, text.clone())),
                _ => None,
            })
            .collect();
        Ok(Self {
            id: stored.id.clone(),
            directory: stored.directory.clone(),
            contents,
            ran,
            notes,
        })
    }

    /// Draws the note that shows at `t`, if one does, over the picture
    /// `pixmap`, with the em `em`.
    pub(crate) fn caption(&self, caption: &mut Caption, t: u64, pixmap: &mut tiny_skia::PixmapMut<'_>, em: f32) {
        if let Some((at, text)) = self.note(t) {
            caption.draw_note(pixmap, em, text, &format!("at {} s of {}", seconds(at), self.id));
        }
    }

    pub(crate) fn has_notes(&self) -> bool {
        !self.notes.is_empty()
    }

    /// The note that shows at `t`, and its time.
    pub(crate) fn note(&self, t: u64) -> Option<(u64, &str)> {
        let shown = self.notes.partition_point(|(at, _)| *at <= t);
        shown.checked_sub(1).map(|index| (self.notes[index].0, self.notes[index].1.as_str()))
    }

    fn video(&self) -> PathBuf {
        self.directory.join(VIDEO)
    }
}

/// What a video asks for.
pub(crate) struct Video<'a> {
    pub parts: &'a [Part],
    /// The coder's library, none on a Host that cannot load it.
    pub load: Option<&'a Load>,
    /// Whether it draws the inputs and the notes.
    pub drawn: bool,
    pub fast_waits: bool,
}

/// What a video came to.
pub(crate) struct Written {
    /// How long it plays, in microseconds.
    pub plays: u64,
    /// What of the notes it could not show as set.
    pub unshown: Vec<String>,
}

/// Whether the pictures of `parts` are of more than one size.
pub(crate) fn resized(parts: &[Part]) -> bool {
    let mut sizes = parts.iter().flat_map(|part| part.contents.frames.iter().map(|kept| (kept.fw, kept.fh)));
    let first = sizes.next();
    sizes.any(|size| Some(size) != first)
}

/// Writes the video to `path`, which is there only once it is whole: a
/// file of its name stays as it was when the export fails.
pub(crate) fn write(path: &Path, video: &Video<'_>) -> Result<Written, Error> {
    let paces: Vec<Pace> = video
        .parts
        .iter()
        .map(|part| if video.fast_waits { Pace::of(&part.contents, part.ran) } else { Pace::even() })
        .collect();
    let overlays: Vec<Overlay> = video.parts.iter().map(|part| Overlay::new(&part.contents.inputs)).collect();
    let over = Over {
        notes: video.drawn && video.parts.iter().any(Part::has_notes),
        badge: video.drawn && paces.iter().any(|pace| !pace.is_even()),
    };
    let drawn = video.drawn && (over.notes || over.badge || overlays.iter().any(|overlay| !overlay.is_empty()));
    let coded = drawn || video.parts.len() > 1 || resized(video.parts);
    let load = match (video.load, coded) {
        (Some(load), true) => Some(load),
        (None, true) if video.parts.len() > 1 || resized(video.parts) => {
            return Err(Error::Unavailable(
                "a video of several recordings, or of a screen that changed size, is coded again, which needs the video coder".into(),
            ));
        }
        _ => None,
    };
    let partial = path.with_file_name(format!(
        ".{}.partial",
        path.file_name().map_or_else(|| "export".into(), |name| name.to_string_lossy())
    ));
    let written = (|| {
        let mut file = std::io::BufWriter::new(fs::File::create(&partial).map_err(|error| Error::io(&partial, &error))?);
        let written = match load {
            Some(load) => {
                let overlays: Vec<Option<&Overlay>> = overlays.iter().map(|overlay| drawn.then_some(overlay)).collect();
                write_drawn(&mut file, video.parts, &paces, &overlays, over, load)?
            }
            None => write_plain(&mut file, video.parts, &paces)?,
        };
        std::io::Write::flush(&mut file).map_err(|error| Error::io(&partial, &error))?;
        fs::rename(&partial, path).map_err(|error| Error::io(path, &error))?;
        Ok(written)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&partial);
    }
    written
}

/// What a drawn video writes over the pictures besides the inputs.
#[derive(Clone, Copy)]
struct Over {
    /// The notes, which a recording of the video has.
    notes: bool,
    /// The badge of a wait played faster, which a part of the video has.
    badge: bool,
}

/// The recordings' own pictures, one after another.
fn write_plain(file: &mut impl std::io::Write, parts: &[Part], paces: &[Pace]) -> Result<Written, Error> {
    let mut videos = parts
        .iter()
        .map(|part| fs::File::open(part.video()).map_err(|error| Error::io(&part.directory, &error)))
        .collect::<Result<Vec<_>, _>>()?;
    // Each sample as its part, its frame, and where the part starts playing.
    let mut samples = Vec::new();
    let mut start = 0;
    for (index, (part, pace)) in parts.iter().zip(paces).enumerate() {
        samples.extend((0..part.contents.frames.len()).map(|frame| (index, frame, start)));
        start += pace.plays(part.ran.max(part.contents.frames.last().map_or(0, |last| last.t + 1000)));
    }
    let mut read = |index: usize| -> Result<mp4::Sample, Error> {
        let (at, frame, start) = samples[index];
        let (part, pace) = (&parts[at], &paces[at]);
        let frames = &part.contents.frames;
        let kept = &frames[frame];
        let mut data = vec![0; kept.l as usize];
        videos[at]
            .seek(SeekFrom::Start(kept.o))
            .and_then(|_| videos[at].read_exact(&mut data))
            .map_err(|error| Error::io(&part.directory, &error))?;
        let next = frames.get(frame + 1).map_or(part.ran.max(kept.t + 1000), |next| next.t);
        Ok(mp4::Sample {
            data,
            key: kept.key,
            duration_us: (start + pace.plays(next)).saturating_sub(start + pace.plays(kept.t)).max(1),
            width: kept.fw,
            height: kept.fh,
        })
    };
    mp4::write(file, samples.len(), &mut read)?;
    Ok(Written { plays: start, unshown: Vec::new() })
}

/// The recordings decoded, drawn on and coded again.
fn write_drawn(
    file: &mut impl std::io::Write,
    parts: &[Part],
    paces: &[Pace],
    overlays: &[Option<&Overlay>],
    over: Over,
    load: &Load,
) -> Result<Written, Error> {
    // A library that cannot load is the Host's, which an export without
    // drawing does without.
    load().map_err(|error| Error::Unavailable(error.to_string()))?;
    let mut caption = Caption::new();
    // The first picture's size.
    let first = &parts[0].contents.frames[0];
    let size = (first.fw + first.fw % 2, first.fh + first.fh % 2);
    let settings = demi_shared_h264::Settings {
        width: size.0,
        height: size.1,
        rate: demi_shared_h264::Rate::Quality,
    };
    let coding = |error: demi_shared_h264::Error| Error::Failed(format!("the drawn video did not code: {error}"));
    // OpenH264 codes each picture before it answers, where a hardware coder
    // works behind its calls and may drop one it has no room for: an export
    // keeps every frame.
    let mut coder = demi_shared_h264::software(settings, load).map_err(coding)?;
    let mut coded: Vec<demi_shared_h264::Picture> = Vec::new();
    let mut picture = Vec::new();
    let mut canvas = vec![0u8; (size.0 * size.1 * 4) as usize];
    let mut start = 0;
    let mut pictures = 0u32;
    for ((part, pace), overlay) in parts.iter().zip(paces).zip(overlays) {
        let mut painter = Painter::new(part, *overlay, load)?;
        let mut times: Vec<u64> = part
            .contents
            .frames
            .iter()
            .map(|kept| kept.t)
            .chain(overlay.map(|overlay| overlay.changes(part.ran)).unwrap_or_default())
            .chain(part.notes.iter().map(|(t, _)| *t))
            .chain(pace.bounds())
            .filter(|t| *t <= part.ran)
            .collect();
        times.sort_unstable();
        times.dedup();
        // The moment the video last showed, so that two moments a wait made
        // one are one picture.
        let mut shown: Option<u64> = None;
        for t in times {
            let plays = start + pace.plays(t);
            painter.advance(t)?;
            if shown.is_some_and(|last| plays <= last) {
                continue;
            }
            let Some(painted) = painter.size() else {
                continue;
            };
            let em = caption::em(painted.0, painted.1);
            painter.paint(t, caption::margin(em), &mut picture);
            if let Some(mut pixmap) = tiny_skia::PixmapMut::from_bytes(&mut picture, painted.0, painted.1) {
                if over.notes {
                    part.caption(&mut caption, t, &mut pixmap, em);
                }
                if over.badge && pace.is_fast(t) {
                    caption.draw_badge(&mut pixmap, em);
                }
            }
            shown = Some(plays);
            centered(&picture, painted, &mut canvas, size);
            let raw = demi_shared_h264::Raw {
                width: size.0,
                height: size.1,
                format: demi_shared_h264::PixelFormat::Rgba,
                pixels: &canvas,
            };
            let key = pictures % crate::KEY_INTERVAL == 0;
            pictures += 1;
            coded.extend(coder.encode(raw, plays, key).map_err(coding)?);
            if !coder.take_dropped().is_empty() {
                return Err(Error::Failed("the drawn video's coder dropped a frame".into()));
            }
        }
        start += pace.plays(part.ran).max(shown.map_or(0, |last| last - start + 1000));
    }
    coded.extend(coder.finish().map_err(coding)?);
    coded.sort_by_key(|picture| picture.time_us);
    if coded.is_empty() {
        return Err(Error::Failed("the drawn video's coder gave no picture".into()));
    }
    let mut read = |index: usize| -> Result<mp4::Sample, Error> {
        let picture = &coded[index];
        let next = coded.get(index + 1).map_or(start, |next| next.time_us);
        Ok(mp4::Sample {
            data: picture.data.clone(),
            key: picture.key,
            duration_us: next.saturating_sub(picture.time_us).max(1),
            width: size.0,
            height: size.1,
        })
    };
    mp4::write(file, coded.len(), &mut read)?;
    Ok(Written {
        plays: start,
        unshown: caption.unshown(),
    })
}

/// Puts `picture`, of `painted` size, in `canvas`, of `size`: as it is when
/// the sizes are one, else scaled to fit, in its middle on black.
fn centered(picture: &[u8], painted: (u32, u32), canvas: &mut [u8], size: (u32, u32)) {
    if painted == size {
        canvas.copy_from_slice(picture);
        return;
    }
    let share = f64::min(f64::from(size.0) / f64::from(painted.0), f64::from(size.1) / f64::from(painted.1));
    let fitted = (
        ((f64::from(painted.0) * share).round() as u32).clamp(1, size.0),
        ((f64::from(painted.1) * share).round() as u32).clamp(1, size.1),
    );
    let source = image::ImageBuffer::<image::Rgba<u8>, &[u8]>::from_raw(painted.0, painted.1, picture)
        .expect("a painted picture has its size's pixels");
    let scaled = image::imageops::resize(&source, fitted.0, fitted.1, image::imageops::FilterType::Triangle);
    for pixel in canvas.chunks_exact_mut(4) {
        pixel.copy_from_slice(&[0, 0, 0, 255]);
    }
    let (width, canvas_width) = (fitted.0 as usize, size.0 as usize);
    let (left, top) = ((size.0 - fitted.0) as usize / 2, (size.1 - fitted.1) as usize / 2);
    for row in 0..fitted.1 as usize {
        let from = row * width * 4;
        let to = ((top + row) * canvas_width + left) * 4;
        canvas[to..to + width * 4].copy_from_slice(&scaled.as_raw()[from..from + width * 4]);
    }
}

/// A recording's screen at any time, decoded from its video in order, with
/// what an export draws on it.
pub(crate) struct Painter<'a> {
    part: &'a Part,
    bytes: Vec<u8>,
    decoder: Decoder,
    /// The next frame to decode.
    next: usize,
    screen: Vec<u8>,
    size: (u32, u32),
    /// The inputs to draw; none for a plain picture.
    overlay: Option<&'a Overlay>,
}

impl<'a> Painter<'a> {
    pub(crate) fn new(part: &'a Part, overlay: Option<&'a Overlay>, load: &Load) -> Result<Self, Error> {
        let bytes = fs::read(part.video()).map_err(|error| Error::io(&part.directory, &error))?;
        let api = load().map_err(|error| Error::Unavailable(error.to_string()))?;
        let decoder = Decoder::with_api_config(api, DecoderConfig::new())
            .map_err(|error| Error::Failed(format!("the video decoder did not start: {error}")))?;
        Ok(Self {
            part,
            bytes,
            decoder,
            next: 0,
            screen: Vec::new(),
            size: (0, 0),
            overlay,
        })
    }

    /// Decodes the frames drawn by `t`.
    pub(crate) fn advance(&mut self, t: u64) -> Result<(), Error> {
        let frames = &self.part.contents.frames;
        while self.next < frames.len() && frames[self.next].t <= t {
            let kept = &frames[self.next];
            let at = kept.o as usize;
            let unit = self
                .bytes
                .get(at..at + kept.l as usize)
                .ok_or_else(|| Error::Corrupt(format!("{} of {} is past the end of its video", frame_label(self.next as u32 + 1), self.part.id)))?;
            if let Some(decoded) = self.decoder.decode(&decodable(unit)).map_err(|error| {
                Error::Corrupt(format!("{} of {} did not decode: {error}", frame_label(self.next as u32 + 1), self.part.id))
            })? {
                if openh264::formats::YUVSource::dimensions(&decoded) != (kept.fw as usize, kept.fh as usize) {
                    return Err(Error::Corrupt(format!(
                        "{} of {} decoded at another size than it was kept",
                        frame_label(self.next as u32 + 1),
                        self.part.id
                    )));
                }
                self.size = (kept.fw, kept.fh);
                self.screen.resize((kept.fw * kept.fh * 4) as usize, 0);
                decoded.write_rgba8(&mut self.screen);
            }
            self.next += 1;
        }
        Ok(())
    }

    /// The size of the screen, none before the first frame.
    pub(crate) fn size(&self) -> Option<(u32, u32)> {
        (!self.screen.is_empty()).then_some(self.size)
    }

    /// Paints the screen as it is at `t` into `canvas`, with the inputs on
    /// it, the key capsule `margin` pixels from the bottom right corner; the
    /// size of the picture, none before the first frame.
    pub(crate) fn paint(&self, t: u64, margin: f32, canvas: &mut Vec<u8>) -> Option<(u32, u32)> {
        let (width, height) = self.size()?;
        canvas.clear();
        canvas.extend_from_slice(&self.screen);
        if let Some(overlay) = self.overlay {
            overlay.draw(t, canvas, width, height, margin);
        }
        Some((width, height))
    }
}

//! The frames of a recording as images, and its video as an MP4.
//!
//! Both draw the inputs the screen took on its frames (`overlay.rs`), unless
//! asked for them plain. A plain MP4 is the video as it was kept; a drawn one
//! is decoded, drawn on and coded again, at the recording's own frames and,
//! while something drawn moves, 30 times a second between them.
//!
//! An image is the frame decoded from the video, cut to the area the export
//! shows, under a strip that names the frame's number and time, so the
//! picture itself holds nothing of the label. The frames of one export share
//! one area, by default all the places any of them changed, so they can be
//! laid side by side.

use std::{
    borrow::Cow,
    fs,
    io::{Read as _, Seek as _, SeekFrom},
    path::{Path, PathBuf},
};

use demi_shared_h264::bitstream::{BitReader, BitWriter, SPS, annex_b, escape, kind, unescape, units};
use demi_shared_openh264::Load;
use demi_shared_recording_protocol::{Area, FrameSet, frame_label};
use image::{ImageEncoder as _, codecs::png::PngEncoder};
use openh264::decoder::{Decoder, DecoderConfig};

use crate::{
    Error, font,
    library::Stored,
    mp4,
    overlay::Overlay,
    stored::{Kept, VIDEO},
};

/// The area of the frames an export shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Crop {
    /// All the places any of the frames changed.
    Changed,
    /// The whole frame.
    Whole,
    Region(Area),
}

/// What an export of images asks for.
pub struct Export<'a> {
    /// The frames to write; every one when none.
    pub frames: Option<&'a FrameSet>,
    pub crop: Crop,
    /// Where the images go.
    pub directory: &'a Path,
    /// Whether an image may replace a file of its name.
    pub overwrite: bool,
    /// The video decoder's library.
    pub load: &'a Load,
    /// Whether the images leave the inputs off the frames.
    pub plain: bool,
}

/// What an export wrote.
pub struct Exported {
    /// The area every image shows.
    pub area: Area,
    pub images: Vec<(u32, PathBuf)>,
}

const SCALE: usize = 2;
const MARGIN: usize = 6;
const STRIP: [u8; 4] = [32, 32, 32, 255];
const TEXT: [u8; 4] = [255, 255, 255, 255];

impl Stored {
    /// Writes the frames as images into the directory. Decoding is work for
    /// a thread of its own.
    pub fn export(&self, export: &Export<'_>) -> Result<Exported, Error> {
        let contents = self.contents()?;
        let total = contents.frames.len() as u32;
        if total == 0 {
            return Err(Error::BadRequest(format!("{} kept no frames", self.id)));
        }
        let chosen: Vec<u32> = (1..=total).filter(|number| export.frames.is_none_or(|set| set.contains(*number))).collect();
        if let Some(set) = export.frames
            && (set.last() > total || chosen.is_empty())
        {
            return Err(Error::BadRequest(format!(
                "{} has frames f001 to {}; {:?} names others",
                self.id,
                frame_label(total),
                String::from(set.clone())
            )));
        }
        // Where the first and the last frame chosen are among the frames.
        let first = chosen[0] as usize - 1;
        let last = *chosen.last().expect("some frame was chosen") as usize - 1;
        let area = match export.crop {
            Crop::Changed => chosen
                .iter()
                .filter_map(|number| {
                    let kept = &contents.frames[*number as usize - 1];
                    kept.changed().within(kept.fw, kept.fh)
                })
                .reduce(Area::union)
                .expect("a frame changed somewhere"),
            Crop::Whole => {
                let kept = &contents.frames[first];
                Area::whole(kept.fw, kept.fh)
            }
            Crop::Region(area) => area,
        };
        fs::create_dir_all(export.directory).map_err(|error| Error::io(export.directory, &error))?;
        // Decoding starts at the key frame at or before the first one wanted.
        let start = (0..=first).rev().find(|index| contents.frames[*index].key).ok_or_else(|| {
            Error::Corrupt(format!("{} has no key frame before {}", self.id, frame_label(chosen[0])))
        })?;
        let mut video = fs::File::open(self.directory.join(VIDEO)).map_err(|error| Error::io(&self.directory, &error))?;
        let (begin, end) = (contents.frames[start].o, contents.frames[last].o + u64::from(contents.frames[last].l));
        video.seek(SeekFrom::Start(begin)).map_err(|error| Error::io(&self.directory, &error))?;
        let mut bytes = vec![0; (end - begin) as usize];
        video.read_exact(&mut bytes).map_err(|error| Error::io(&self.directory, &error))?;
        let api = (export.load)().map_err(|error| Error::Unavailable(error.to_string()))?;
        let mut decoder = Decoder::with_api_config(api, DecoderConfig::new())
            .map_err(|error| Error::Failed(format!("the video decoder did not start: {error}")))?;
        let overlay = Overlay::new(&contents.inputs);
        let drawn = !export.plain && !overlay.is_empty();
        let mut images = Vec::new();
        for index in start..=last {
            let kept = &contents.frames[index];
            let at = (kept.o - begin) as usize;
            let label = || format!("{} of {}", frame_label(index as u32 + 1), self.id);
            let decoded = decoder
                .decode(&decodable(&bytes[at..at + kept.l as usize]))
                .map_err(|error| Error::Corrupt(format!("{} did not decode: {error}", label())))?
                .ok_or_else(|| Error::Corrupt(format!("{} decoded to no picture", label())))?;
            let number = index as u32 + 1;
            if !chosen.contains(&number) {
                continue;
            }
            let (width, height) = (kept.fw as usize, kept.fh as usize);
            let mut rgba = vec![0; width * height * 4];
            if openh264::formats::YUVSource::dimensions(&decoded) != (width, height) {
                return Err(Error::Corrupt(format!("{} decoded at another size than it was kept", frame_label(number))));
            }
            decoded.write_rgba8(&mut rgba);
            if drawn {
                overlay.draw(kept.t, &mut rgba, kept.fw, kept.fh);
            }
            let shown = area.within(kept.fw, kept.fh).ok_or_else(|| {
                Error::BadRequest(format!(
                    "the area {area} is outside {}, which is {}x{} pixels",
                    frame_label(number),
                    kept.fw,
                    kept.fh
                ))
            })?;
            let path = export.directory.join(format!("{}.png", frame_label(number)));
            if !export.overwrite && path.exists() {
                return Err(Error::Exists(path));
            }
            let label = format!("{} {:.3} s", frame_label(number), kept.t as f64 / 1_000_000.0);
            fs::write(&path, labelled(&rgba, width, shown, &label)?).map_err(|error| Error::io(&path, &error))?;
            images.push((number, path));
        }
        Ok(Exported { area, images })
    }

    /// Writes the recording's video as an MP4 to `path`, with its inputs
    /// drawn on it when `drawn` gives the coder's library, and as it was kept
    /// when it does not or the recording took no input.
    pub fn write_mp4(&self, path: &Path, drawn: Option<&Load>) -> Result<std::time::Duration, Error> {
        let contents = self.contents()?;
        if contents.frames.is_empty() {
            return Err(Error::BadRequest(format!("{} kept no frames", self.id)));
        }
        let ran = match (&contents.meta.ended, &self.live) {
            (Some(ended), _) => ended.duration_us,
            (None, Some(live)) => live.info()?.duration_ms * 1000,
            (None, None) => contents.frames.last().map_or(0, |last| last.t),
        };
        let overlay = Overlay::new(&contents.inputs);
        if let Some(load) = drawn
            && !overlay.is_empty()
        {
            return self.write_drawn(path, &contents, &overlay, ran, load);
        }
        let mut video = fs::File::open(self.directory.join(VIDEO)).map_err(|error| Error::io(&self.directory, &error))?;
        let frames = &contents.frames;
        let mut file = std::io::BufWriter::new(fs::File::create(path).map_err(|error| Error::io(path, &error))?);
        let mut read = |index: usize| -> Result<mp4::Sample, Error> {
            let kept: &Kept = &frames[index];
            let mut data = vec![0; kept.l as usize];
            video
                .seek(SeekFrom::Start(kept.o))
                .and_then(|_| video.read_exact(&mut data))
                .map_err(|error| Error::io(&self.directory, &error))?;
            let next = frames.get(index + 1).map_or(ran.max(kept.t + 1000), |next| next.t);
            Ok(mp4::Sample {
                data,
                key: kept.key,
                duration_us: next.saturating_sub(kept.t).max(1),
                width: kept.fw,
                height: kept.fh,
            })
        };
        mp4::write(&mut file, frames.len(), &mut read)?;
        std::io::Write::flush(&mut file).map_err(|error| Error::io(path, &error))?;
        Ok(std::time::Duration::from_micros(ran))
    }
}

impl Stored {
    /// Writes the video decoded, drawn on by `overlay` and coded again: a
    /// frame at each of the recording's own frames, and at each time the
    /// overlay changes, each lasting until the next.
    fn write_drawn(
        &self,
        path: &Path,
        contents: &crate::stored::Contents,
        overlay: &Overlay,
        ran: u64,
        load: &Load,
    ) -> Result<std::time::Duration, Error> {
        let frames = &contents.frames;
        let bytes = fs::read(self.directory.join(VIDEO)).map_err(|error| Error::io(&self.directory, &error))?;
        let mut times: Vec<u64> = frames.iter().map(|kept| kept.t).chain(overlay.changes(ran)).collect();
        times.sort_unstable();
        times.dedup();
        let api = load().map_err(|error| Error::Unavailable(error.to_string()))?;
        let mut decoder = Decoder::with_api_config(api, DecoderConfig::new())
            .map_err(|error| Error::Failed(format!("the video decoder did not start: {error}")))?;
        let mut encoder: Option<(Box<dyn demi_shared_h264::Encoder>, (u32, u32))> = None;
        let mut screen: Vec<u8> = Vec::new();
        let mut size = (0, 0);
        let mut drawn = Vec::new();
        let mut coded: Vec<demi_shared_h264::Picture> = Vec::new();
        let mut sizes = std::collections::BTreeMap::new();
        let mut next = 0;
        let coding = |error: demi_shared_h264::Error| Error::Failed(format!("the drawn video did not code: {error}"));
        for (index, &t) in times.iter().enumerate() {
            while next < frames.len() && frames[next].t <= t {
                let kept = &frames[next];
                let at = kept.o as usize;
                if let Some(decoded) = decoder
                    .decode(&decodable(&bytes[at..at + kept.l as usize]))
                    .map_err(|error| Error::Corrupt(format!("{} of {} did not decode: {error}", frame_label(next as u32 + 1), self.id)))?
                {
                    size = (kept.fw, kept.fh);
                    screen.resize((kept.fw * kept.fh * 4) as usize, 0);
                    decoded.write_rgba8(&mut screen);
                }
                next += 1;
            }
            if screen.is_empty() {
                continue;
            }
            drawn.clear();
            drawn.extend_from_slice(&screen);
            overlay.draw(t, &mut drawn, size.0, size.1);
            // A frame of another size starts a coder of its own, at a key frame.
            let restart = encoder.as_ref().is_none_or(|(_, at)| *at != size);
            if restart {
                if let Some((mut old, _)) = encoder.take() {
                    coded.extend(old.finish().map_err(coding)?);
                }
                let settings = demi_shared_h264::Settings {
                    width: size.0,
                    height: size.1,
                    rate: demi_shared_h264::Rate::Quality,
                };
                // OpenH264 codes each picture before it answers, where a
                // hardware coder works behind its calls and may drop one it
                // has no room for: an export keeps every frame.
                encoder = Some((Box::new(demi_shared_h264::software(settings, load).map_err(coding)?), size));
            }
            let (coder, _) = encoder.as_mut().expect("a coder was started");
            sizes.insert(t, size);
            let raw = demi_shared_h264::Raw {
                width: size.0,
                height: size.1,
                format: demi_shared_h264::PixelFormat::Rgba,
                pixels: &drawn,
            };
            let key = restart || index as u32 % crate::KEY_INTERVAL == 0;
            coded.extend(coder.encode(raw, t, key).map_err(coding)?);
            if !coder.take_dropped().is_empty() {
                return Err(Error::Failed("the drawn video's coder dropped a frame".into()));
            }
        }
        if let Some((mut coder, _)) = encoder.take() {
            coded.extend(coder.finish().map_err(coding)?);
        }
        coded.sort_by_key(|picture| picture.time_us);
        if coded.is_empty() {
            return Err(Error::Failed("the drawn video's coder gave no picture".into()));
        }
        let mut file = std::io::BufWriter::new(fs::File::create(path).map_err(|error| Error::io(path, &error))?);
        let mut read = |index: usize| -> Result<mp4::Sample, Error> {
            let picture = &coded[index];
            let next = coded.get(index + 1).map_or(ran.max(picture.time_us + 1000), |next| next.time_us);
            let (width, height) = sizes.range(..=picture.time_us).next_back().map_or(size, |(_, size)| *size);
            Ok(mp4::Sample {
                data: picture.data.clone(),
                key: picture.key,
                duration_us: next.saturating_sub(picture.time_us).max(1),
                width,
                height,
            })
        };
        mp4::write(&mut file, coded.len(), &mut read)?;
        std::io::Write::flush(&mut file).map_err(|error| Error::io(path, &error))?;
        Ok(std::time::Duration::from_micros(ran))
    }
}

/// The frame's Annex B bytes, with the SPS rewritten so that OpenH264 2.6.0
/// predicts a frame from the picture it names.
///
/// A hardware encoder may mark pictures as long-term references (MMCO 6) and
/// have the next frame name one by `long_term_pic_num`. OpenH264 orders the
/// references of a CABAC stream that is not Baseline by a routine that
/// compares that number with one it never sets for a picture marked so, finds
/// none and keeps the default order, so the frame is predicted from the key
/// frame, and everything drawn since is lost. Its routine for Baseline finds
/// the picture, and the decoder picks the routine by `profile_idc` alone, so
/// a High or Main SPS is named Baseline. A High SPS loses the fields Baseline
/// has not, which are dropped when they hold the defaults (4:2:0, 8 bits, no
/// scaling matrix); any other SPS is left as it is, and said.
///
/// Baseline also makes OpenH264 output in decoding order, not in display
/// order. That holds while no source makes B frames, as none does.
fn decodable(frame: &[u8]) -> Cow<'_, [u8]> {
    let units = units(frame);
    let mut changed = false;
    let rewritten: Vec<Cow<'_, [u8]>> = units
        .iter()
        .map(|unit| {
            // A stream that is Baseline already needs nothing.
            if kind(unit) != Some(SPS) || unit.get(1) == Some(&BASELINE) {
                return Cow::Borrowed(*unit);
            }
            match baseline(unit) {
                Some(sps) => {
                    changed = true;
                    Cow::Owned(sps)
                }
                None => {
                    tracing::warn!(
                        profile = unit.get(1).copied(),
                        "an SPS could not be named Baseline, so a frame that names a long-term reference may decode wrongly"
                    );
                    Cow::Borrowed(*unit)
                }
            }
        })
        .collect();
    if changed { Cow::Owned(annex_b(&rewritten)) } else { Cow::Borrowed(frame) }
}

/// `profile_idc` of the Baseline profile, which OpenH264 orders long-term
/// references right for.
const BASELINE: u8 = 66;

/// The SPS `unit` as a Baseline one, when it is a Main or High SPS this can
/// be done to.
fn baseline(unit: &[u8]) -> Option<Vec<u8>> {
    // chroma_format_idc 1 (`010`), bit depths 8 (`1`, `1`), no lossless
    // bypass (`0`), no scaling matrix (`0`).
    const HIGH_DEFAULTS: u32 = 0b010_1_1_0_0;
    let (&header, payload) = unit.split_first()?;
    let mut rbsp = unescape(payload);
    let mut reader = BitReader::new(&rbsp);
    let profile = reader.bits(8)?;
    reader.bits(16)?; // constraint flags, level_idc
    reader.ue()?; // seq_parameter_set_id
    match profile {
        77 => {}
        100 => {
            let fields = reader.position();
            if reader.bits(7)? != HIGH_DEFAULTS {
                return None;
            }
            let mut writer = BitWriter::default();
            writer.copy(&rbsp, 0..fields);
            writer.copy(&rbsp, fields + 7..rbsp.len() * 8);
            rbsp = writer.finish();
            // Zero bytes after the stop bit are not part of the unit.
            while rbsp.last() == Some(&0) {
                rbsp.pop();
            }
        }
        _ => return None,
    }
    *rbsp.first_mut()? = BASELINE;
    let mut sps = vec![header];
    escape(&rbsp, &mut sps);
    Some(sps)
}

/// The PNG of `area` of `rgba` under a strip that says `label`.
fn labelled(rgba: &[u8], width: usize, area: Area, label: &str) -> Result<Vec<u8>, Error> {
    let (text_width, text_height) = font::size(label, SCALE);
    let strip = text_height + 2 * MARGIN;
    let (area_width, area_height) = (area.width as usize, area.height as usize);
    let canvas_width = area_width.max(text_width + 2 * MARGIN);
    let mut canvas = vec![0u8; canvas_width * (strip + area_height) * 4];
    for pixel in canvas.chunks_exact_mut(4) {
        pixel.copy_from_slice(&STRIP);
    }
    font::draw(&mut canvas, canvas_width, MARGIN, MARGIN, label, SCALE, TEXT);
    for row in 0..area_height {
        let from = ((area.y as usize + row) * width + area.x as usize) * 4;
        let to = ((strip + row) * canvas_width) * 4;
        canvas[to..to + area_width * 4].copy_from_slice(&rgba[from..from + area_width * 4]);
    }
    // The picture is opaque.
    for pixel in canvas.chunks_exact_mut(4) {
        pixel[3] = 255;
    }
    let mut png = Vec::new();
    PngEncoder::new(&mut png)
        .write_image(&canvas, canvas_width as u32, (strip + area_height) as u32, image::ExtendedColorType::Rgba8)
        .map_err(|error| Error::Failed(format!("an image did not encode: {error}")))?;
    Ok(png)
}

//! H.264 encoding of a screen's pictures, on the Host's hardware where it has
//! an encoder (`docs/decisions/0036-a-recording-keeps-every-drawn-frame.md`).
//!
//! A source of raw pictures, an Android emulator's screen, hands them to an
//! [`Encoder`], which gives back coded pictures: H.264 Annex B, one access
//! unit each, with the parameter sets in front of every key frame. Two ways of
//! coding serve the two uses of a screen:
//!
//! - [`Rate::Quality`] codes every picture at a fixed quality that keeps a
//!   one-pixel move visible, and skips none for a bit rate: a recording. A
//!   picture an encoder has no room for is dropped all the same, and said
//!   so ([`Encoder::take_dropped`]), never failed.
//! - [`Rate::Bitrate`] holds a bit rate, which a picture that does not fit may
//!   be skipped for: the live view.
//!
//! ```text
//! open(settings) -> hardware encoder of this system, if it starts
//!                   |      VideoToolbox   macOS
//!                   |      Media Foundation   Windows
//!                   |      VA-API   Linux
//!                   +--> else OpenH264 in software, Cisco's build
//! ```
//!
//! # Boundaries
//!
//! - The crate codes pictures and knows no source, no recording and no live
//!   view. What to do with the coded pictures is the caller's.
//! - Hardware encoders are optional on every system. A Host where
//!   [`hardware`] says no codes in software, which keeps up with a phone's
//!   logical size and not with its pixels, so the caller that has the choice
//!   of size asks first.

#[cfg(windows)]
mod mediafoundation;
#[cfg(windows)]
mod nv12;
mod software;
#[cfg(target_os = "macos")]
mod videotoolbox;

pub use software::{Software, software};

use std::sync::OnceLock;

use demi_shared_openh264::Load;

/// Why a picture could not be coded.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{0}")]
pub struct Error(pub String);

pub type Result<T> = std::result::Result<T, Error>;

/// The order of a raw picture's bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    Rgb,
    Rgba,
    Bgra,
}

impl PixelFormat {
    pub fn bytes(self) -> usize {
        match self {
            Self::Rgb => 3,
            Self::Rgba | Self::Bgra => 4,
        }
    }
}

/// A raw picture: rows from the top, no padding between them.
#[derive(Debug, Clone, Copy)]
pub struct Raw<'a> {
    pub width: u32,
    pub height: u32,
    pub format: PixelFormat,
    pub pixels: &'a [u8],
}

/// What an encoder holds on to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Rate {
    /// Every picture at a fixed quality, none skipped.
    Quality,
    /// A bit rate and a frame rate, which a picture over the budget is
    /// skipped for.
    Bitrate { bits_per_second: u32, fps: u32 },
}

/// How an encoder is started. Both sides are even: 4:2:0 has no odd side.
#[derive(Debug, Clone, Copy)]
pub struct Settings {
    pub width: u32,
    pub height: u32,
    pub rate: Rate,
}

/// A coded picture.
#[derive(Debug, Clone)]
pub struct Picture {
    /// H.264 Annex B, one access unit, with the SPS and PPS in front of a
    /// key frame.
    pub data: Vec<u8>,
    pub key: bool,
    /// The time the picture was given with, in microseconds.
    pub time_us: u64,
}

/// One encoder at one size.
pub trait Encoder: Send {
    /// What codes the pictures, for a log.
    fn name(&self) -> &'static str;

    /// Codes `frame`, drawn at `time_us`, as a key frame when `key`. Answers
    /// the pictures that are ready, which may be earlier ones, since a
    /// hardware encoder works behind the call; none when the picture was
    /// skipped for the bit rate.
    fn encode(&mut self, frame: Raw<'_>, time_us: u64, key: bool) -> Result<Vec<Picture>>;

    /// Moves a [`Rate::Bitrate`] encoder to another bit rate and frame rate.
    fn set_rate(&mut self, bits_per_second: u32, fps: u32) -> Result<()>;

    /// Waits for the pictures the encoder still holds and answers them.
    fn finish(&mut self) -> Result<Vec<Picture>>;

    /// The times of the pictures the encoder dropped since this was last
    /// asked, and so will never answer: a [`Rate::Bitrate`] encoder skips
    /// them for its budget, and any encoder drops one it has no room for. A
    /// recording counts them as missed and goes on.
    fn take_dropped(&mut self) -> Vec<u64>;
}

/// Starts the encoder of `settings`: the Host's hardware one where it starts,
/// else OpenH264, which `load` loads.
pub fn open(settings: Settings, load: &Load) -> Result<Box<dyn Encoder>> {
    if settings.width < 2 || settings.height < 2 || settings.width % 2 != 0 || settings.height % 2 != 0 {
        return Err(Error(format!("H.264 needs even sides of at least two pixels, not {} × {}", settings.width, settings.height)));
    }
    if hardware() {
        match hardware_encoder(settings) {
            Ok(encoder) => return Ok(encoder),
            Err(error) => tracing::warn!("the hardware encoder did not start, so OpenH264 codes: {error}"),
        }
    }
    Ok(Box::new(software::software(settings, load)?))
}

/// Whether this Host has a hardware encoder that works, found once.
pub fn hardware() -> bool {
    static FOUND: OnceLock<bool> = OnceLock::new();
    *FOUND.get_or_init(|| {
        let probe = Settings {
            width: 128,
            height: 128,
            rate: Rate::Quality,
        };
        let found = hardware_encoder(probe).is_ok();
        tracing::debug!(found, "whether this Host has a hardware H.264 encoder");
        found
    })
}

#[cfg(target_os = "macos")]
fn hardware_encoder(settings: Settings) -> Result<Box<dyn Encoder>> {
    Ok(Box::new(videotoolbox::VideoToolbox::start(settings)?))
}

#[cfg(windows)]
fn hardware_encoder(settings: Settings) -> Result<Box<dyn Encoder>> {
    Ok(Box::new(mediafoundation::MediaFoundation::start(settings)?))
}

#[cfg(not(any(target_os = "macos", windows)))]
fn hardware_encoder(_settings: Settings) -> Result<Box<dyn Encoder>> {
    Err(Error("this system has no hardware H.264 encoder here".into()))
}

#[cfg(test)]
mod tests {
    use demi_shared_openh264::{Built, Codec as _};
    use openh264::{
        decoder::{Decoder, DecoderConfig},
        formats::YUVSource as _,
    };

    use super::*;

    const WIDTH: u32 = 128;
    const HEIGHT: u32 = 96;
    const SIDE: u32 = 400;

    /// A gradient with a red square at column `step * 8`.
    fn picture(step: u32) -> Vec<u8> {
        let mut pixels = Vec::with_capacity((WIDTH * HEIGHT * 4) as usize);
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                let inside = (step * 8..step * 8 + 16).contains(&x) && (40..56).contains(&y);
                let color = if inside { [230, 20, 20, 255] } else { [(x * 2) as u8, (y * 2) as u8, 120, 255] };
                pixels.extend_from_slice(&color);
            }
        }
        pixels
    }

    fn raw(pixels: &[u8]) -> Raw<'_> {
        Raw {
            width: WIDTH,
            height: HEIGHT,
            format: PixelFormat::Rgba,
            pixels,
        }
    }

    /// The mean distance of a decoded picture's colors from the drawn ones.
    fn distance(coded: &Picture, drawn: &[u8], decoder: &mut Decoder) -> f64 {
        let decoded = decoder.decode(&coded.data).unwrap().expect("a coded picture decodes to a picture");
        assert_eq!(decoded.dimensions(), (WIDTH as usize, HEIGHT as usize));
        let mut rgba = vec![0; drawn.len()];
        decoded.write_rgba8(&mut rgba);
        let total: u64 = rgba.chunks_exact(4).zip(drawn.chunks_exact(4)).map(|(a, b)| (0..3).map(|c| u64::from(a[c].abs_diff(b[c]))).sum::<u64>()).sum();
        total as f64 / f64::from(WIDTH * HEIGHT * 3)
    }

    /// What an encoder codes decodes, in order, to what was drawn, and the
    /// first picture is a key frame that carries its parameter sets.
    async fn codes_what_was_drawn(mut encoder: Box<dyn Encoder>) {
        let api = (Built.library().await.unwrap())().unwrap();
        let mut decoder = Decoder::with_api_config(api, DecoderConfig::new()).unwrap();
        let frames: Vec<Vec<u8>> = (0..12).map(picture).collect();
        let mut coded = Vec::new();
        for (step, frame) in frames.iter().enumerate() {
            coded.extend(encoder.encode(raw(frame), step as u64 * 16_667, step == 0).unwrap());
        }
        coded.extend(encoder.finish().unwrap());
        assert_eq!(coded.len(), frames.len(), "{} coded every picture", encoder.name());
        assert!(coded[0].key && coded[0].data.starts_with(&[0, 0, 0, 1]), "the first picture is a key frame");
        assert!(coded.iter().skip(1).all(|picture| !picture.key), "only the first was asked to be a key frame");
        let times: Vec<u64> = coded.iter().map(|picture| picture.time_us).collect();
        assert_eq!(times, (0..12).map(|step| step * 16_667).collect::<Vec<u64>>(), "each picture keeps its time");
        for (picture, drawn) in coded.iter().zip(&frames) {
            let far = distance(picture, drawn, &mut decoder);
            assert!(far < 4.0, "{} codes a picture {far:.1} levels from what was drawn", encoder.name());
        }
    }

    /// Planted defects this catches: pictures that come out of order or
    /// fewer, a first picture that is no key frame or lacks its parameter
    /// sets, and a time that is not the one given.
    ///
    /// Cost: twelve pictures of 128 × 96, milliseconds.
    #[tokio::test]
    async fn the_software_encoder_codes_what_was_drawn() {
        let load = Built.library().await.unwrap();
        let settings = Settings {
            width: WIDTH,
            height: HEIGHT,
            rate: Rate::Quality,
        };
        codes_what_was_drawn(Box::new(software(settings, &load).unwrap())).await;
    }

    /// The Host's hardware encoder, where it has one, codes the same.
    #[tokio::test]
    async fn the_hardware_encoder_codes_what_was_drawn() {
        let settings = Settings {
            width: WIDTH,
            height: HEIGHT,
            rate: Rate::Quality,
        };
        match hardware_encoder(settings) {
            Ok(encoder) => codes_what_was_drawn(encoder).await,
            Err(error) => eprintln!("no hardware encoder on this Host: {error}"),
        }
    }

    /// The pictures an encoder of [`Rate::Quality`] codes show a change of one
    /// pixel: a white pixel drawn on a picture that did not have it decodes
    /// clearly brighter than the pixel did before.
    async fn a_one_pixel_change_shows(mut encoder: Box<dyn Encoder>) {
        let api = (Built.library().await.unwrap())().unwrap();
        let mut decoder = Decoder::with_api_config(api, DecoderConfig::new()).unwrap();
        let background = |mark: bool| {
            let mut pixels = Vec::with_capacity((SIDE * SIDE * 4) as usize);
            for y in 0..SIDE {
                for x in 0..SIDE {
                    let color = if mark && (x, y) == (203, 151) { [255, 255, 255, 255] } else { [(x / 2) as u8, (y / 2) as u8, 90, 255] };
                    pixels.extend_from_slice(&color);
                }
            }
            pixels
        };
        fn big(pixels: &[u8]) -> Raw<'_> {
            Raw { width: SIDE, height: SIDE, format: PixelFormat::Rgba, pixels }
        }
        let (plain, marked) = (background(false), background(true));
        let mut coded = Vec::new();
        for (step, pixels) in [&plain, &plain, &marked].into_iter().enumerate() {
            coded.extend(encoder.encode(big(pixels), step as u64 * 16_667, step == 0).unwrap());
        }
        coded.extend(encoder.finish().unwrap());
        let mut lumas = Vec::new();
        for picture in &coded {
            let decoded = decoder.decode(&picture.data).unwrap().expect("a coded picture decodes to a picture");
            let mut rgba = vec![0; (SIDE * SIDE * 4) as usize];
            decoded.write_rgba8(&mut rgba);
            let at = ((151 * SIDE + 203) * 4) as usize;
            lumas.push(u32::from(rgba[at]) + u32::from(rgba[at + 1]) + u32::from(rgba[at + 2]));
        }
        assert_eq!(lumas.len(), 3);
        eprintln!("{}: the pixel decodes to {lumas:?} (white is 765)", encoder.name());
        assert!(lumas[2] > lumas[1] + 300, "{} shows a one-pixel change as {lumas:?}", encoder.name());
    }

    /// Planted defect this catches: a quality at which a one-pixel change is
    /// smoothed away, which a recording meant to keep every change would lose.
    #[tokio::test]
    async fn a_recording_encoder_shows_a_change_of_one_pixel() {
        let load = Built.library().await.unwrap();
        let settings = Settings {
            width: 400,
            height: 400,
            rate: Rate::Quality,
        };
        a_one_pixel_change_shows(Box::new(software(settings, &load).unwrap())).await;
        if let Ok(encoder) = hardware_encoder(settings) {
            a_one_pixel_change_shows(encoder).await;
        }
    }

    /// Every picture given to an encoder is answered or said dropped, and a
    /// dropped one is told by its time.
    ///
    /// Planted defect this catches: a skipped picture that is neither given
    /// back nor told, which a recording could not count as missed.
    #[tokio::test]
    async fn every_picture_is_given_back_or_said_dropped() {
        let load = Built.library().await.unwrap();
        let settings = Settings {
            width: WIDTH,
            height: HEIGHT,
            rate: Rate::Bitrate {
                bits_per_second: 1_000,
                fps: 30,
            },
        };
        let mut encoder = software(settings, &load).unwrap();
        let mut given = Vec::new();
        for step in 0..30 {
            let noise: Vec<u8> = (0..WIDTH * HEIGHT * 4).map(|at| (at.wrapping_mul(2_654_435_761).wrapping_add(step * 40_503) >> 13) as u8).collect();
            given.extend(encoder.encode(raw(&noise), u64::from(step) * 33_333, step == 0).unwrap());
        }
        given.extend(encoder.finish().unwrap());
        let dropped = encoder.take_dropped();
        assert!(!dropped.is_empty(), "a budget this small skips pictures");
        let mut times: Vec<u64> = given.iter().map(|picture| picture.time_us).chain(dropped).collect();
        times.sort_unstable();
        assert_eq!(times, (0..30).map(|step| step * 33_333).collect::<Vec<u64>>());
        assert!(encoder.take_dropped().is_empty(), "the drops are told once");
    }

    /// A bit rate and frame rate are taken, and a picture over the budget may
    /// be skipped, never a key frame asked for.
    #[tokio::test]
    async fn a_live_encoder_takes_its_rate_and_a_key_frame_when_asked() {
        let load = Built.library().await.unwrap();
        let settings = Settings {
            width: WIDTH,
            height: HEIGHT,
            rate: Rate::Bitrate {
                bits_per_second: 2_000_000,
                fps: 30,
            },
        };
        let mut encoder = open(settings, &load).unwrap();
        let mut coded = Vec::new();
        for step in 0..8 {
            coded.extend(encoder.encode(raw(&picture(step)), u64::from(step) * 33_333, step == 0).unwrap());
        }
        encoder.set_rate(1_000_000, 15).unwrap();
        coded.extend(encoder.encode(raw(&picture(8)), 8 * 33_333, true).unwrap());
        coded.extend(encoder.finish().unwrap());
        assert!(coded.first().is_some_and(|first| first.key), "{} starts with a key frame", encoder.name());
        assert!(coded.last().is_some_and(|last| last.key), "the key frame asked for after the rate changed comes");
    }
}

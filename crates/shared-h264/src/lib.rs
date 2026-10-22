//! H.264 encoding of a screen's pictures, on the Host's hardware where it has
//! an encoder.
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
            Err(error) => tracing::warn!(
                event = "h264_encoder.fallback",
                to = "openh264",
                %error,
                "the hardware encoder did not start, so OpenH264 codes"
            ),
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
        tracing::debug!(event = "h264_encoder.probed", hardware = found, "whether this Host has a hardware H.264 encoder was found");
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

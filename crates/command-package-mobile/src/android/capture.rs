//! One capture of the live view for an emulator
//! (`crates/command-package-mobile/src/android/mod.rs`):
//! the emulator's pictures at the size the view asked for, which the
//! emulator scales and turns upright, encoded with OpenH264 as H.264,
//! Annex B, in real time. The emulator sends a picture only when its screen
//! changed, so a still screen sends no frame; a key frame asked for is sent
//! at once, of the picture as it is.

use std::{
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};

use bytes::Bytes;
use futures_util::future::BoxFuture;
use openh264::{
    encoder::{BitRate, Encoder, EncoderConfig, FrameRate, FrameType, RateControlMode, UsageType},
    formats::{RgbSliceU8, YUVBuffer, YUVSource as _},
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use demi_live_view::stream::{CaptureEvent, EncodedFrame};

use super::{
    emulator::Client,
    openh264::Load,
    proto::{ImageFormat, image_format::ImgFormat},
};
use crate::driver::{DeviceCapture, DriverError, Result};

/// One picture from the emulator: RGB, rows from the top.
struct Picture {
    rgb: Bytes,
    width: usize,
    height: usize,
}

/// What the encoder's thread is asked to do.
#[derive(Default)]
struct Asked {
    /// The newest picture not encoded yet.
    newest: Option<Arc<Picture>>,
    key_frame: bool,
    encoding: Option<(u32, u32)>,
    /// The newest frame the viewers had, and how many more their windows
    /// have room for.
    allowed: Option<(u32, u32)>,
    stopped: bool,
}

impl Asked {
    /// Whether the viewers' windows have room for the frame after `sequence`;
    /// before they said, for one.
    fn room(&self, sequence: u32) -> bool {
        let (floor, window) = self.allowed.unwrap_or((0, 1));
        sequence.saturating_sub(floor) < window
    }
}

struct Shared {
    asked: Mutex<Asked>,
    wake: Condvar,
}

impl Shared {
    fn ask(&self, change: impl FnOnce(&mut Asked)) {
        change(&mut self.asked.lock().expect("the capture's state is intact"));
        self.wake.notify_one();
    }
}

/// A running capture; dropping it stops it.
pub struct AndroidCapture {
    frames: mpsc::Receiver<CaptureEvent>,
    shared: Arc<Shared>,
    stop: CancellationToken,
}

impl AndroidCapture {
    /// Starts capturing the emulator's pictures at most `size`, encoding
    /// them with the library `load` reaches.
    pub fn start(mut client: Client, load: Load, size: (u32, u32), fps: u32, bitrate: u32) -> Self {
        let (events, frames) = mpsc::channel(8);
        let shared = Arc::new(Shared {
            asked: Mutex::new(Asked::default()),
            wake: Condvar::new(),
        });
        let stop = CancellationToken::new();
        let encoder = shared.clone();
        let encoded = events.clone();
        std::thread::Builder::new()
            .name("demi-mobile-h264".into())
            .spawn(move || encode(&encoder, &encoded, &load, fps, bitrate))
            .expect("the encoder's thread starts");
        let pictures = shared.clone();
        let stopping = stop.clone();
        tokio::spawn(async move {
            let request = ImageFormat {
                format: ImgFormat::Rgb888.into(),
                width: size.0,
                height: size.1,
                ..ImageFormat::default()
            };
            let streamed = async {
                let mut stream = client
                    .stream_screenshot(request)
                    .await
                    .map_err(|status| status.message().to_owned())?
                    .into_inner();
                while let Some(image) = stream.message().await.map_err(|status| status.message().to_owned())? {
                    let (width, height) = image
                        .format
                        .as_ref()
                        .map_or((0, 0), |format| (format.width as usize, format.height as usize));
                    // An empty picture says the display is off.
                    if width < 2 || height < 2 || image.image.len() < width * height * 3 {
                        continue;
                    }
                    let picture = Arc::new(Picture {
                        rgb: image.image,
                        width,
                        height,
                    });
                    pictures.ask(|asked| asked.newest = Some(picture));
                }
                Ok::<(), String>(())
            };
            tokio::select! {
                () = stopping.cancelled() => {}
                streamed = streamed => {
                    let reason = match streamed {
                        Ok(()) => "the emulator stopped sending its screen".to_owned(),
                        Err(reason) => format!("the emulator's screen: {reason}"),
                    };
                    // The view hears of it unless it left already.
                    let _ = events.try_send(CaptureEvent::Failed(reason));
                }
            }
            pictures.ask(|asked| asked.stopped = true);
        });
        Self { frames, shared, stop }
    }
}

impl DeviceCapture for AndroidCapture {
    fn next(&mut self) -> BoxFuture<'_, Option<CaptureEvent>> {
        Box::pin(self.frames.recv())
    }

    fn ack(&self, sequence: u32, window: u32) {
        self.shared.ask(|asked| asked.allowed = Some((sequence, window)));
    }

    fn key_frame(&self) {
        self.shared.ask(|asked| asked.key_frame = true);
    }

    fn encoding(&self, bitrate: u32, fps: u32) {
        self.shared.ask(|asked| asked.encoding = Some((bitrate, fps)));
    }
}

impl Drop for AndroidCapture {
    fn drop(&mut self) {
        self.stop.cancel();
        self.shared.ask(|asked| asked.stopped = true);
    }
}

fn encoder(load: &Load, bitrate: u32, fps: u32) -> Result<Encoder> {
    let config = EncoderConfig::new()
        .usage_type(UsageType::ScreenContentRealTime)
        .rate_control_mode(RateControlMode::Bitrate)
        // Neither works for screen content, which OpenH264 says each time
        // an encoder starts.
        .adaptive_quantization(false)
        .background_detection(false)
        .bitrate(BitRate::from_bps(bitrate))
        .max_frame_rate(FrameRate::from_hz(fps.max(1) as f32))
        // Rate control holds the bit rate only by skipping pictures; a
        // skipped one is encoded again at the next interval, so the last
        // picture of a change still shows.
        .skip_frames(true);
    Encoder::with_api_config(load()?, config)
        .map_err(|error| DriverError::Failed(format!("the video encoder did not start: {error}")))
}

/// The encoder's thread: encodes the newest picture at most `fps` times a
/// second, so the last picture of a change is always sent, until stopped.
fn encode(shared: &Shared, events: &mpsc::Sender<CaptureEvent>, load: &Load, fps: u32, bitrate: u32) {
    let mut interval = Duration::from_secs_f64(1.0 / f64::from(fps.max(1)));
    let mut current = match encoder(load, bitrate, fps) {
        Ok(encoder) => encoder,
        Err(error) => {
            let _ = events.try_send(CaptureEvent::Failed(error.to_string()));
            return;
        }
    };
    let started = Instant::now();
    let mut last: Option<Arc<Picture>> = None;
    let mut encoded_at: Option<Instant> = None;
    let mut sequence: u32 = 0;
    let mut force_key = true;
    let mut yuv: Option<YUVBuffer> = None;
    loop {
        let picture = {
            let mut asked = shared.asked.lock().expect("the capture's state is intact");
            loop {
                if asked.stopped {
                    return;
                }
                if let Some((bitrate, fps)) = asked.encoding.take() {
                    // OpenH264's safe interface sets the bit rate only as
                    // an encoder starts: a new one takes over, from a key
                    // frame.
                    match encoder(load, bitrate, fps) {
                        Ok(encoder) => {
                            current = encoder;
                            force_key = true;
                            interval = Duration::from_secs_f64(1.0 / f64::from(fps.max(1)));
                        }
                        Err(error) => tracing::warn!("the encoding did not change: {error}"),
                    }
                }
                if asked.key_frame {
                    asked.key_frame = false;
                    force_key = true;
                    if asked.newest.is_none() {
                        asked.newest = last.clone();
                    }
                }
                // Pictures come no faster than the interval allows.
                let due = encoded_at.map_or(Duration::ZERO, |at| interval.saturating_sub(at.elapsed()));
                // The newest picture waits for room in the viewers' windows,
                // replaced by any newer one.
                let room = asked.room(sequence);
                if asked.newest.is_some() && due.is_zero() && room {
                    break asked.newest.take().expect("a picture is waiting");
                }
                asked = if asked.newest.is_some() && room {
                    shared.wake.wait_timeout(asked, due).expect("the capture's state is intact").0
                } else {
                    shared.wake.wait(asked).expect("the capture's state is intact")
                };
            }
        };
        encoded_at = Some(Instant::now());
        last = Some(picture.clone());
        // H.264 needs even sides: an odd last row or column is left out.
        let (width, height) = (picture.width & !1, picture.height & !1);
        let rgb = even_rgb(&picture, width, height);
        let buffer = match &mut yuv {
            Some(buffer) if buffer.dimensions() == (width, height) => buffer,
            slot => slot.insert(YUVBuffer::new(width, height)),
        };
        buffer.read_rgb8(RgbSliceU8::new(&rgb, (width, height)));
        if force_key {
            current.force_intra_frame();
            force_key = false;
        }
        let stream = match current.encode(buffer) {
            Ok(stream) => stream,
            Err(error) => {
                let _ = events.try_send(CaptureEvent::Failed(format!("a frame did not encode: {error}")));
                return;
            }
        };
        let key = matches!(stream.frame_type(), FrameType::IDR | FrameType::I);
        let data = stream.to_vec();
        if stream.frame_type() == FrameType::Skip || data.is_empty() {
            shared.ask(|asked| {
                asked.newest.get_or_insert(picture);
            });
            continue;
        }
        sequence = sequence.wrapping_add(1);
        let data = Bytes::from(data);
        // A stream that has not taken the frame before is behind; its
        // viewers resume from a key frame.
        let _behind = events.try_send(CaptureEvent::Frame(EncodedFrame {
            sequence,
            key,
            timestamp: started.elapsed().as_secs_f64() * 1_000_000.0,
            width: width as u16,
            height: height as u16,
            data: if key { crate::pictures::without_reordering(data) } else { data },
        }));
    }
}

/// The picture's RGB cut to `width` × `height`, its own bytes when it is
/// that size already.
fn even_rgb(picture: &Picture, width: usize, height: usize) -> std::borrow::Cow<'_, [u8]> {
    if width == picture.width && height == picture.height {
        return std::borrow::Cow::Borrowed(&picture.rgb[..width * height * 3]);
    }
    let mut cut = Vec::with_capacity(width * height * 3);
    for row in picture.rgb.chunks_exact(picture.width * 3).take(height) {
        cut.extend_from_slice(&row[..width * 3]);
    }
    std::borrow::Cow::Owned(cut)
}

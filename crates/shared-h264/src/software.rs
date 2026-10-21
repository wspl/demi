//! OpenH264, Cisco's build, in software: the encoder of a Host without
//! hardware's, and the one the tests use.

use openh264::{
    encoder::{BitRate, Complexity, Encoder, EncoderConfig, FrameRate, FrameType, IntraFramePeriod, RateControlMode, UsageType},
    formats::{BgraSliceU8, RgbSliceU8, RgbaSliceU8, YUVBuffer},
};

use demi_shared_openh264::Load;

use crate::{Error, Picture, PixelFormat, Raw, Rate, Result, Settings};

/// The quantizer every picture of [`Rate::Quality`] is coded at, in H.264's 0
/// to 51 where lower keeps more. At 18 a one-pixel move of text shows, and a
/// whole screen moving costs about 12 KiB a picture at a phone's size.
const QUANTIZER: i32 = 18;

/// OpenH264 in software, whatever the Host's hardware: for a Host without
/// an encoder of its own, and for tests that must code the same everywhere.
pub fn software(settings: Settings, load: &Load) -> Result<Software> {
    Software::start(settings, load)
}

pub struct Software {
    encoder: Encoder,
    load: Load,
    settings: Settings,
    /// The next picture is coded from a key frame: the encoder is new.
    restarted: bool,
    yuv: YUVBuffer,
    /// Whether the encoder has started: its first picture is coded again
    /// from a key frame once its quantizer is set.
    tuned: bool,
    /// The times of the pictures the encoder skipped.
    dropped: Vec<u64>,
}

impl Software {
    pub fn start(settings: Settings, load: &Load) -> Result<Self> {
        let api = load().map_err(|error| Error(error.to_string()))?;
        let encoder = Encoder::with_api_config(api, config(settings.rate))
            .map_err(|error| Error(format!("the video encoder did not start: {error}")))?;
        Ok(Self {
            encoder,
            load: load.clone(),
            settings,
            restarted: false,
            yuv: YUVBuffer::new(settings.width as usize, settings.height as usize),
            tuned: false,
            dropped: Vec::new(),
        })
    }
}

fn config(rate: Rate) -> EncoderConfig {
    match rate {
        // Screen content mode keeps text sharp; no rate control, so that a
        // picture costs what its quantizer says and none is skipped.
        Rate::Quality => EncoderConfig::new()
            .usage_type(UsageType::ScreenContentRealTime)
            .rate_control_mode(RateControlMode::Off)
            .max_frame_rate(FrameRate::from_hz(60.0))
            .skip_frames(false)
            .scene_change_detect(true)
            .adaptive_quantization(false)
            .background_detection(false)
            .complexity(Complexity::Low)
            .intra_frame_period(IntraFramePeriod::from_num_frames(0)),
        Rate::Bitrate { bits_per_second, fps } => EncoderConfig::new()
            .usage_type(UsageType::ScreenContentRealTime)
            .rate_control_mode(RateControlMode::Bitrate)
            // Neither works for screen content, which OpenH264 says each time
            // an encoder starts.
            .adaptive_quantization(false)
            .background_detection(false)
            .bitrate(BitRate::from_bps(bits_per_second))
            .max_frame_rate(FrameRate::from_hz(fps.max(1) as f32))
            // Rate control holds the bit rate only by skipping pictures; a
            // skipped one is coded again at the next interval, so the last
            // picture of a change still shows.
            .skip_frames(true),
    }
}

impl crate::Encoder for Software {
    fn name(&self) -> &'static str {
        "OpenH264"
    }

    fn encode(&mut self, frame: Raw<'_>, time_us: u64, key: bool) -> Result<Vec<Picture>> {
        let (width, height) = (frame.width as usize, frame.height as usize);
        if (frame.width, frame.height) != (self.settings.width, self.settings.height)
            || frame.pixels.len() != width * height * frame.format.bytes()
        {
            return Err(Error("a picture is not the size the encoder was started at".into()));
        }
        match frame.format {
            PixelFormat::Rgb => self.yuv.read_rgb8(RgbSliceU8::new(frame.pixels, (width, height))),
            PixelFormat::Rgba => self.yuv.read_rgba8(RgbaSliceU8::new(frame.pixels, (width, height))),
            PixelFormat::Bgra => self.yuv.read_bgra8(BgraSliceU8::new(frame.pixels, (width, height))),
        }
        if key || self.restarted {
            self.encoder.force_intra_frame();
            self.restarted = false;
        }
        if !self.tuned && self.settings.rate == Rate::Quality {
            // The first picture starts the encoder, which only then takes its
            // quantizer; the picture is coded again from a key frame with it.
            self.encoder
                .encode(&self.yuv)
                .map_err(|error| Error(format!("a picture did not encode: {error}")))?;
            fix_quantizer(&mut self.encoder)?;
            self.encoder.force_intra_frame();
        }
        self.tuned = true;
        let stream = self
            .encoder
            .encode(&self.yuv)
            .map_err(|error| Error(format!("a picture did not encode: {error}")))?;
        let data = stream.to_vec();
        if stream.frame_type() == FrameType::Skip || data.is_empty() {
            self.dropped.push(time_us);
            return Ok(Vec::new());
        }
        let key = key || matches!(stream.frame_type(), FrameType::IDR | FrameType::I);
        Ok(vec![Picture { data, key, time_us }])
    }

    fn set_rate(&mut self, bits_per_second: u32, fps: u32) -> Result<()> {
        // OpenH264's safe interface sets the bit rate only as an encoder
        // starts: a new one takes over, from a key frame.
        self.settings.rate = Rate::Bitrate { bits_per_second, fps };
        let api = (self.load)().map_err(|error| Error(error.to_string()))?;
        self.encoder = Encoder::with_api_config(api, config(self.settings.rate))
            .map_err(|error| Error(format!("the video encoder did not start: {error}")))?;
        self.restarted = true;
        Ok(())
    }

    fn finish(&mut self) -> Result<Vec<Picture>> {
        Ok(Vec::new())
    }

    fn take_dropped(&mut self) -> Vec<u64> {
        std::mem::take(&mut self.dropped)
    }
}

/// Sets a started encoder to code every picture at [`QUANTIZER`]. The safe
/// interface sets a quantizer only as a range that rate control moves within.
fn fix_quantizer(encoder: &mut Encoder) -> Result<()> {
    use openh264_sys2::{ENCODER_OPTION_SVC_ENCODE_PARAM_EXT, RC_OFF_MODE, SEncParamExt};
    let mut params = SEncParamExt::default();
    // SAFETY: the encoder is initialized, and this option reads and writes
    // exactly a `SEncParamExt`.
    let failed = unsafe {
        let api = encoder.raw_api();
        let read = api.get_option(ENCODER_OPTION_SVC_ENCODE_PARAM_EXT, std::ptr::addr_of_mut!(params).cast());
        params.iRCMode = RC_OFF_MODE;
        params.bEnableFrameSkip = false;
        params.sSpatialLayers[0].iDLayerQp = QUANTIZER;
        read != 0 || api.set_option(ENCODER_OPTION_SVC_ENCODE_PARAM_EXT, std::ptr::addr_of_mut!(params).cast()) != 0
    };
    if failed {
        return Err(Error("the video encoder refused its quantizer".into()));
    }
    Ok(())
}

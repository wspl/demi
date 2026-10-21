//! The hardware encoder of a Windows Host: the video encoder transform its GPU
//! driver registers with Media Foundation (NVENC, Quick Sync, AMF), which takes
//! NV12 and gives H.264.
//!
//! These transforms are asynchronous: the transform says through events when it
//! wants a picture and when one is coded, so a call to [`encode`](crate::Encoder::encode)
//! waits for its turn to give a picture, and answers the pictures that are
//! ready.
//!
//! Only a hardware transform is used: Windows' own software encoder is slower
//! than OpenH264, which codes where there is no hardware.

use std::collections::VecDeque;

use windows::{
    Win32::{
        Foundation::{RPC_E_CHANGED_MODE, S_FALSE},
        Media::MediaFoundation::{
            CODECAPI_AVEncCommonQuality, CODECAPI_AVEncCommonRateControlMode, CODECAPI_AVEncMPVDefaultBPictureCount,
            CODECAPI_AVEncMPVGOPSize, CODECAPI_AVEncVideoForceKeyFrame, CODECAPI_AVLowLatencyMode, ICodecAPI,
            IMFActivate, IMFMediaEventGenerator, IMFMediaType, IMFSample, IMFTransform, MEError, METransformDrainComplete,
            METransformHaveOutput, METransformNeedInput, MF_E_NO_EVENTS_AVAILABLE, MF_E_TRANSFORM_NEED_MORE_INPUT,
            MF_EVENT_FLAG_NO_WAIT, MF_MT_AVG_BITRATE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE, MF_MT_INTERLACE_MODE,
            MF_MT_MAJOR_TYPE, MF_MT_MPEG2_PROFILE, MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SUBTYPE, MF_TRANSFORM_ASYNC,
            MF_TRANSFORM_ASYNC_UNLOCK, MF_VERSION, MFCreateMediaType, MFCreateMemoryBuffer, MFCreateSample,
            MFMediaType_Video, MFSTARTUP_FULL, MFStartup, MFT_CATEGORY_VIDEO_ENCODER, MFT_ENUM_FLAG_HARDWARE,
            MFT_ENUM_FLAG_SORTANDFILTER, MFT_FRIENDLY_NAME_Attribute, MFT_MESSAGE_COMMAND_DRAIN, MFT_MESSAGE_NOTIFY_BEGIN_STREAMING,
            MFT_MESSAGE_NOTIFY_START_OF_STREAM, MFT_OUTPUT_DATA_BUFFER, MFT_OUTPUT_STREAM_PROVIDES_SAMPLES,
            MFT_REGISTER_TYPE_INFO, MFTEnumEx, MFVideoFormat_H264, MFVideoFormat_NV12, MFVideoInterlace_Progressive,
        },
        System::{
            Com::{CoInitializeEx, CoTaskMemFree, COINIT_MULTITHREADED},
            Variant::VARIANT,
        },
    },
    core::{GUID, Interface as _},
};

use crate::{Error, Picture, Raw, Rate, Result, Settings, nv12::Nv12};

/// 100-nanosecond units to the microsecond.
const TICKS: i64 = 10;
/// The quality every picture of [`Rate::Quality`] is coded at, 0 to 100.
const QUALITY: u32 = 90;
/// A key frame every this many pictures.
const KEY_INTERVAL: u32 = 120;
/// H.264 High profile.
const HIGH: u32 = 100;
/// `eAVEncCommonRateControlMode_Quality` and `_UnconstrainedVBR`, `_CBR`.
const QUALITY_MODE: u32 = 3;
const CBR_MODE: u32 = 0;

fn failed(what: &str) -> impl Fn(windows::core::Error) -> Error + '_ {
    move |error| Error(format!("Media Foundation: {what} failed: {error}"))
}

/// Starts COM and Media Foundation for the calling thread, which every call
/// into the encoder does first: a thread that already did is left as it is.
fn start_thread() -> Result<()> {
    // SAFETY: plain initialization of the calling thread.
    let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    if com.is_err() && com != S_FALSE && com != RPC_E_CHANGED_MODE {
        return Err(Error(format!("COM did not start: {com:?}")));
    }
    // SAFETY: startup counts its callers; the process never shuts it down.
    unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL) }.map_err(failed("its startup"))
}

pub struct MediaFoundation {
    settings: Settings,
    transform: IMFTransform,
    codec: ICodecAPI,
    events: IMFMediaEventGenerator,
    nv12: Nv12,
    /// How many pictures the transform asked for and has not been given.
    wanted: u32,
    ready: VecDeque<Picture>,
    /// The times of the pictures the transform had no room for.
    dropped: Vec<u64>,
    /// The parameter sets of the last key frame, which every key frame
    /// carries in front.
    headers: Vec<u8>,
    draining: bool,
}

// SAFETY: the transform is free-threaded (a hardware encoder transform is
// asynchronous and takes calls from any thread of the multithreaded
// apartment), and the encoder is used by one thread at a time.
unsafe impl Send for MediaFoundation {}

impl MediaFoundation {
    pub fn start(settings: Settings) -> Result<Self> {
        start_thread()?;
        let mut refused = Vec::new();
        for (name, transform) in hardware_transforms()? {
            match Self::configured(settings, transform) {
                Ok(encoder) => return Ok(encoder),
                Err(error) => refused.push(format!("{name}: {error}")),
            }
        }
        Err(Error(format!("no hardware H.264 encoder starts here: {}", refused.join("; "))))
    }

    fn configured(settings: Settings, transform: IMFTransform) -> Result<Self> {
        // SAFETY: the transform is valid; the attribute store is its own.
        unsafe {
            let attributes = transform.GetAttributes().map_err(failed("its attributes"))?;
            if attributes.GetUINT32(&MF_TRANSFORM_ASYNC).unwrap_or(0) != 0 {
                attributes.SetUINT32(&MF_TRANSFORM_ASYNC_UNLOCK, 1).map_err(failed("unlocking its events"))?;
            }
        }
        let codec: ICodecAPI = transform.cast().map_err(failed("its codec interface"))?;
        let events: IMFMediaEventGenerator = transform.cast().map_err(failed("its events"))?;
        let fps = match settings.rate {
            Rate::Bitrate { fps, .. } => fps.max(1),
            Rate::Quality => 60,
        };
        // The width and height by the area of the screen, and a bit rate that
        // gives a quality-mode encoder that cannot honour its mode room to
        // keep every picture.
        let bits = match settings.rate {
            Rate::Bitrate { bits_per_second, .. } => bits_per_second,
            Rate::Quality => (settings.width * settings.height).saturating_mul(fps).clamp(1_000_000, 200_000_000),
        };
        // SAFETY: the media types are created and set on the transform they
        // describe, which owns them after.
        unsafe {
            let output = media_type(&MFVideoFormat_H264, settings, fps)?;
            output.SetUINT32(&MF_MT_AVG_BITRATE, bits).map_err(failed("the bit rate"))?;
            output.SetUINT32(&MF_MT_MPEG2_PROFILE, HIGH).map_err(failed("the profile"))?;
            transform.SetOutputType(0, &output, 0).map_err(failed("the output type"))?;
            let input = media_type(&MFVideoFormat_NV12, settings, fps)?;
            transform.SetInputType(0, &input, 0).map_err(failed("the input type"))?;
        }
        // None of these is required of a transform: one that refuses a setting
        // codes with its own.
        set(&codec, &CODECAPI_AVLowLatencyMode, 1);
        set(&codec, &CODECAPI_AVEncMPVDefaultBPictureCount, 0);
        set(&codec, &CODECAPI_AVEncMPVGOPSize, KEY_INTERVAL);
        match settings.rate {
            Rate::Quality => {
                set(&codec, &CODECAPI_AVEncCommonRateControlMode, QUALITY_MODE);
                set(&codec, &CODECAPI_AVEncCommonQuality, QUALITY);
            }
            Rate::Bitrate { .. } => {
                set(&codec, &CODECAPI_AVEncCommonRateControlMode, CBR_MODE);
            }
        }
        // SAFETY: the transform is configured.
        unsafe {
            transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)
                .map_err(failed("beginning the stream"))?;
            transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)
                .map_err(failed("starting the stream"))?;
        }
        Ok(Self {
            settings,
            transform,
            codec,
            events,
            nv12: Nv12::new(settings.width, settings.height),
            wanted: 0,
            ready: VecDeque::new(),
            dropped: Vec::new(),
            headers: Vec::new(),
            draining: false,
        })
    }

    /// Takes what the transform says until it has nothing more to say now, or,
    /// with `until_input`, until it asks for a picture.
    fn pump(&mut self, until_input: bool) -> Result<()> {
        loop {
            if until_input && self.wanted > 0 {
                return Ok(());
            }
            let flags = if until_input && self.wanted == 0 && !self.draining {
                windows::Win32::Media::MediaFoundation::MEDIA_EVENT_GENERATOR_GET_EVENT_FLAGS(0)
            } else {
                MF_EVENT_FLAG_NO_WAIT
            };
            // SAFETY: the generator is the transform's.
            let event = match unsafe { self.events.GetEvent(flags) } {
                Ok(event) => event,
                Err(error) if error.code() == MF_E_NO_EVENTS_AVAILABLE => return Ok(()),
                Err(error) => return Err(failed("an event")(error)),
            };
            // SAFETY: the event is valid.
            let kind = unsafe { event.GetType() }.map_err(failed("an event's type"))?;
            if kind == METransformNeedInput.0 as u32 {
                self.wanted += 1;
            } else if kind == METransformHaveOutput.0 as u32 {
                self.output()?;
            } else if kind == METransformDrainComplete.0 as u32 {
                self.draining = false;
                return Ok(());
            } else if kind == MEError.0 as u32 {
                // SAFETY: the event is valid.
                let status = unsafe { event.GetStatus() }.map_err(failed("an event's status"))?;
                return Err(Error(format!("the encoder failed: {status:?}")));
            }
        }
    }

    /// Takes the picture the transform has ready.
    fn output(&mut self) -> Result<()> {
        // SAFETY: the transform is valid; the buffer is the transform's to fill
        // or ours when it does not provide samples.
        unsafe {
            let info = self.transform.GetOutputStreamInfo(0).map_err(failed("the output's info"))?;
            let provides = info.dwFlags & MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32 != 0;
            let sample = if provides {
                None
            } else {
                let sample = MFCreateSample().map_err(failed("a sample"))?;
                let buffer = MFCreateMemoryBuffer(info.cbSize.max(1 << 20)).map_err(failed("a buffer"))?;
                sample.AddBuffer(&buffer).map_err(failed("a buffer"))?;
                Some(sample)
            };
            let mut data = [MFT_OUTPUT_DATA_BUFFER {
                dwStreamID: 0,
                pSample: std::mem::ManuallyDrop::new(sample),
                dwStatus: 0,
                pEvents: std::mem::ManuallyDrop::new(None),
            }];
            let mut status = 0;
            let result = self.transform.ProcessOutput(0, &mut data, &mut status);
            let sample = std::mem::ManuallyDrop::take(&mut data[0].pSample);
            drop(std::mem::ManuallyDrop::take(&mut data[0].pEvents));
            match result {
                Ok(()) => {}
                Err(error) if error.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => return Ok(()),
                Err(error) => return Err(failed("an output")(error)),
            }
            let Some(sample) = sample else {
                return Ok(());
            };
            let picture = self.picture(&sample)?;
            self.ready.push_back(picture);
        }
        Ok(())
    }

    fn picture(&mut self, sample: &IMFSample) -> Result<Picture> {
        // SAFETY: the sample is valid; the buffer is locked while read.
        let (bytes, time) = unsafe {
            let buffer = sample.ConvertToContiguousBuffer().map_err(failed("an output's buffer"))?;
            let (mut data, mut length) = (std::ptr::null_mut(), 0);
            buffer.Lock(&mut data, None, Some(&mut length)).map_err(failed("an output's lock"))?;
            let bytes = std::slice::from_raw_parts(data, length as usize).to_vec();
            buffer.Unlock().map_err(failed("an output's unlock"))?;
            (bytes, sample.GetSampleTime().map_err(failed("an output's time"))?)
        };
        let mut data = bytes;
        let key = units(&data).any(|unit| unit & 0x1f == 5);
        if key {
            if units(&data).any(|unit| unit & 0x1f == 7) {
                self.headers = parameter_sets(&data);
            } else if !self.headers.is_empty() {
                let mut with = self.headers.clone();
                with.extend_from_slice(&data);
                data = with;
            }
        }
        Ok(Picture {
            data,
            key,
            time_us: u64::try_from(time / TICKS).unwrap_or(0),
        })
    }
}

/// The first byte of each NAL unit of an Annex B stream.
fn units(data: &[u8]) -> impl Iterator<Item = u8> + '_ {
    data.windows(4)
        .enumerate()
        .filter(|(at, window)| window[..3] == [0, 0, 1] || (window[..3] == [0, 0, 0] && data.get(at + 3) == Some(&1)))
        .filter_map(|(at, window)| if window[2] == 1 { data.get(at + 3).copied() } else { data.get(at + 4).copied() })
}

/// The SPS and PPS units of an Annex B key frame, start codes included.
fn parameter_sets(data: &[u8]) -> Vec<u8> {
    let mut starts: Vec<usize> = Vec::new();
    let mut at = 0;
    while at + 3 <= data.len() {
        if data[at..at + 3] == [0, 0, 1] {
            // The start code may be four bytes.
            starts.push(if at > 0 && data[at - 1] == 0 { at - 1 } else { at });
            at += 3;
        } else {
            at += 1;
        }
    }
    let mut sets = Vec::new();
    for (index, start) in starts.iter().enumerate() {
        let end = starts.get(index + 1).copied().unwrap_or(data.len());
        let code = if data[*start] == 0 && data[*start + 1] == 0 && data[*start + 2] == 0 { 4 } else { 3 };
        if matches!(data.get(start + code).map(|byte| byte & 0x1f), Some(7 | 8)) {
            sets.extend_from_slice(&data[*start..end]);
        }
    }
    sets
}

fn media_type(subtype: &GUID, settings: Settings, fps: u32) -> Result<IMFMediaType> {
    // SAFETY: the type is created here and filled before it is handed on.
    unsafe {
        let kind = MFCreateMediaType().map_err(failed("a media type"))?;
        kind.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video).map_err(failed("a media type"))?;
        kind.SetGUID(&MF_MT_SUBTYPE, subtype).map_err(failed("a media type"))?;
        kind.SetUINT64(&MF_MT_FRAME_SIZE, pair(settings.width, settings.height)).map_err(failed("a media type"))?;
        kind.SetUINT64(&MF_MT_FRAME_RATE, pair(fps, 1)).map_err(failed("a media type"))?;
        kind.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pair(1, 1)).map_err(failed("a media type"))?;
        kind.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32).map_err(failed("a media type"))?;
        Ok(kind)
    }
}

/// Two numbers as an attribute carries them: one in each half.
fn pair(high: u32, low: u32) -> u64 {
    (u64::from(high) << 32) | u64::from(low)
}

/// Sets a setting of the codec, when it takes it.
fn set(codec: &ICodecAPI, setting: &GUID, value: u32) {
    // SAFETY: the setting is a GUID of Media Foundation and the value a
    // variant that outlives the call.
    let _refused = unsafe { codec.SetValue(setting, &VARIANT::from(value)) };
}

/// The hardware video encoder transforms that take NV12 and give H.264, by
/// the driver's name, best first.
fn hardware_transforms() -> Result<Vec<(String, IMFTransform)>> {
    let input = MFT_REGISTER_TYPE_INFO {
        guidMajorType: MFMediaType_Video,
        guidSubtype: MFVideoFormat_NV12,
    };
    let output = MFT_REGISTER_TYPE_INFO {
        guidMajorType: MFMediaType_Video,
        guidSubtype: MFVideoFormat_H264,
    };
    let mut found: *mut Option<IMFActivate> = std::ptr::null_mut();
    let mut count = 0;
    let mut transforms = Vec::new();
    // SAFETY: the enumeration fills `found` with `count` activation objects
    // that this function takes and the array it frees.
    unsafe {
        MFTEnumEx(
            MFT_CATEGORY_VIDEO_ENCODER,
            MFT_ENUM_FLAG_HARDWARE | MFT_ENUM_FLAG_SORTANDFILTER,
            Some(&input),
            Some(&output),
            &mut found,
            &mut count,
        )
        .map_err(failed("listing the encoders"))?;
        for index in 0..count as usize {
            let Some(activate) = std::ptr::read(found.add(index)) else {
                continue;
            };
            let mut name = [0u16; 128];
            let mut length = 0;
            let named = activate.GetString(&MFT_FRIENDLY_NAME_Attribute, &mut name, Some(&mut length)).is_ok();
            let name = if named { String::from_utf16_lossy(&name[..length as usize]) } else { "an encoder".into() };
            if let Ok(made) = activate.ActivateObject::<IMFTransform>() {
                transforms.push((name, made));
            }
        }
        CoTaskMemFree(Some(found.cast()));
    }
    if transforms.is_empty() {
        return Err(Error("this Host has no hardware H.264 encoder".into()));
    }
    Ok(transforms)
}

impl crate::Encoder for MediaFoundation {
    fn name(&self) -> &'static str {
        "Media Foundation"
    }

    fn encode(&mut self, frame: Raw<'_>, time_us: u64, key: bool) -> Result<Vec<Picture>> {
        start_thread()?;
        let (width, height) = (frame.width as usize, frame.height as usize);
        if (frame.width, frame.height) != (self.settings.width, self.settings.height)
            || frame.pixels.len() != width * height * frame.format.bytes()
        {
            return Err(Error("a picture is not the size the encoder was started at".into()));
        }
        self.nv12.read(frame)?;
        // A picture waits for the transform to ask for it.
        self.pump(true)?;
        if self.wanted == 0 {
            // The transform is behind: this picture is dropped, and counted.
            self.dropped.push(time_us);
            return Ok(self.ready.drain(..).collect());
        }
        let ((luma, luma_stride), (chroma, chroma_stride)) = self.nv12.planes();
        let size = width * height * 3 / 2;
        // SAFETY: the buffer is locked while the planes are copied into it,
        // row by row, into `size` bytes; the sample goes to the transform.
        unsafe {
            let buffer = MFCreateMemoryBuffer(size as u32).map_err(failed("a buffer"))?;
            let (mut data, mut max) = (std::ptr::null_mut(), 0);
            buffer.Lock(&mut data, Some(&mut max), None).map_err(failed("a buffer's lock"))?;
            let rows = std::slice::from_raw_parts_mut(data, size);
            for row in 0..height {
                rows[row * width..(row + 1) * width].copy_from_slice(&luma[row * luma_stride..row * luma_stride + width]);
            }
            for row in 0..height / 2 {
                let to = width * height + row * width;
                rows[to..to + width].copy_from_slice(&chroma[row * chroma_stride..row * chroma_stride + width]);
            }
            buffer.Unlock().map_err(failed("a buffer's unlock"))?;
            buffer.SetCurrentLength(size as u32).map_err(failed("a buffer's length"))?;
            let sample = MFCreateSample().map_err(failed("a sample"))?;
            sample.AddBuffer(&buffer).map_err(failed("a buffer"))?;
            sample.SetSampleTime(i64::try_from(time_us).unwrap_or(i64::MAX) * TICKS).map_err(failed("a sample's time"))?;
            let fps = match self.settings.rate {
                Rate::Bitrate { fps, .. } => fps.max(1),
                Rate::Quality => 60,
            };
            sample.SetSampleDuration(10_000_000 / i64::from(fps)).map_err(failed("a sample's duration"))?;
            if key {
                let _refused = self.codec.SetValue(&CODECAPI_AVEncVideoForceKeyFrame, &VARIANT::from(1u32));
            }
            self.transform.ProcessInput(0, &sample, 0).map_err(failed("a picture"))?;
        }
        self.wanted -= 1;
        self.pump(false)?;
        Ok(self.ready.drain(..).collect())
    }

    fn set_rate(&mut self, bits_per_second: u32, fps: u32) -> Result<()> {
        self.settings.rate = Rate::Bitrate { bits_per_second, fps };
        // SAFETY: the codec is valid; a transform that does not take the
        // bit rate while running keeps its own.
        let _refused = unsafe {
            self.codec.SetValue(&windows::Win32::Media::MediaFoundation::CODECAPI_AVEncCommonMeanBitRate, &VARIANT::from(bits_per_second))
        };
        Ok(())
    }

    fn finish(&mut self) -> Result<Vec<Picture>> {
        start_thread()?;
        self.draining = true;
        // SAFETY: the transform is valid.
        unsafe {
            self.transform.ProcessMessage(MFT_MESSAGE_COMMAND_DRAIN, 0).map_err(failed("draining"))?;
        }
        while self.draining {
            self.pump(false)?;
            if self.draining {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        Ok(self.ready.drain(..).collect())
    }

    fn take_dropped(&mut self) -> Vec<u64> {
        std::mem::take(&mut self.dropped)
    }
}

//! What an audio file holds, read from its header: the one reader of a
//! generated audio file's format and length, which `demi audio` checks the
//! file with and the usage ledger takes its seconds from (`audio.md`
//! § Commands, `usage-and-quota.md` § Usage ledger). The file is probed and
//! read packet by packet, never decoded, behind the `audio` feature; the
//! shape of raw PCM, which a vendor may send instead of a file, is read by
//! every crate that hands audio on.

#[cfg(feature = "audio")]
use std::io::Cursor;

#[cfg(feature = "audio")]
use symphonia::core::codecs::CodecParameters;
#[cfg(feature = "audio")]
use symphonia::core::codecs::audio::well_known::{CODEC_ID_OPUS, CODEC_ID_VORBIS};
#[cfg(feature = "audio")]
use symphonia::core::formats::probe::Hint;
#[cfg(feature = "audio")]
use symphonia::core::formats::well_known::{FORMAT_ID_FLAC, FORMAT_ID_MP3, FORMAT_ID_OGG, FORMAT_ID_WAVE};
#[cfg(feature = "audio")]
use symphonia::core::formats::{FormatOptions, Track};
#[cfg(feature = "audio")]
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
#[cfg(feature = "audio")]
use symphonia::core::meta::MetadataOptions;

/// The formats a vendor answers with that this reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioFormat {
    Mp3,
    Wav,
    Flac,
    /// Opus in an Ogg container.
    Opus,
    /// Vorbis in an Ogg container.
    Vorbis,
}

impl AudioFormat {
    /// As a call's line names it, such as `MP3`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Mp3 => "MP3",
            Self::Wav => "WAV",
            Self::Flac => "FLAC",
            Self::Opus => "Opus",
            Self::Vorbis => "Ogg Vorbis",
        }
    }
}

/// What the header says of the audio.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AudioFacts {
    pub format: AudioFormat,
    /// How long it plays, in seconds.
    pub seconds: f64,
}

/// Why bytes are not an audio file this reads.
#[cfg(feature = "audio")]
#[derive(Debug, thiserror::Error)]
pub enum NotAudio {
    #[error("the file is no MP3, WAV, FLAC or Ogg audio: {0}")]
    Unreadable(symphonia::core::errors::Error),
    #[error("the file has no audio track")]
    NoAudio,
    #[error("the file's {0} track is not one this reads")]
    Codec(&'static str),
    #[error("the file does not say how long it plays")]
    NoLength,
}

/// The facts of the audio file `bytes`: a container this reads with an
/// audio track whose length its header states or its packets add up to.
#[cfg(feature = "audio")]
pub fn read(bytes: &bytes::Bytes) -> Result<AudioFacts, NotAudio> {
    // A clone of `Bytes` shares the file rather than copying it.
    let source = MediaSourceStream::new(Box::new(Cursor::new(bytes.clone())), MediaSourceStreamOptions::default());
    let mut reader = symphonia::default::get_probe()
        .probe(&Hint::new(), source, FormatOptions::default(), MetadataOptions::default())
        .map_err(NotAudio::Unreadable)?;
    let container = reader.format_info().format;
    let track = reader
        .tracks()
        .iter()
        .find(|track| matches!(track.codec_params, Some(CodecParameters::Audio(_))))
        .cloned()
        .ok_or(NotAudio::NoAudio)?;
    let Some(CodecParameters::Audio(codec)) = &track.codec_params else {
        return Err(NotAudio::NoAudio);
    };
    let format = match container {
        FORMAT_ID_MP3 => AudioFormat::Mp3,
        FORMAT_ID_WAVE => AudioFormat::Wav,
        FORMAT_ID_FLAC => AudioFormat::Flac,
        FORMAT_ID_OGG if codec.codec == CODEC_ID_OPUS => AudioFormat::Opus,
        FORMAT_ID_OGG if codec.codec == CODEC_ID_VORBIS => AudioFormat::Vorbis,
        FORMAT_ID_OGG => return Err(NotAudio::Codec("Ogg")),
        _ => return Err(NotAudio::Codec(reader.format_info().short_name)),
    };
    if let Some(seconds) = stated(&track, codec.sample_rate) {
        return Ok(AudioFacts { format, seconds });
    }
    // A header that states no length, such as an MP3's without a Xing
    // frame: its packets' lengths add up to it.
    let time_base = track.time_base.ok_or(NotAudio::NoLength)?;
    let mut ticks: u64 = 0;
    while let Some(packet) = reader.next_packet().map_err(NotAudio::Unreadable)? {
        if packet.track_id == track.id {
            ticks += packet.dur.get();
        }
    }
    let seconds = time_base
        .calc_duration(symphonia::core::units::Duration::new(ticks))
        .ok_or(NotAudio::NoLength)?
        .as_secs_f64();
    Ok(AudioFacts { format, seconds })
}

/// The length `track`'s header states: its duration in its time base,
/// else its frames at its sample rate.
#[cfg(feature = "audio")]
fn stated(track: &Track, sample_rate: Option<u32>) -> Option<f64> {
    if let (Some(duration), Some(time_base)) = (track.duration, track.time_base)
        && let Some(time) = time_base.calc_duration(duration)
    {
        return Some(time.as_secs_f64());
    }
    let (frames, rate) = (track.num_frames?, sample_rate?);
    (rate > 0).then(|| frames as f64 / f64::from(rate))
}

/// Raw PCM as a vendor sends it, without a header: signed little-endian
/// samples, interleaved by channel. Every value is checked when it is
/// made, so its rate, channels and sample size are ones a WAV header holds
/// and a player plays (`audio.md` § Vendors).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pcm {
    sample_rate: u32,
    channels: u16,
    bits: u16,
}

/// Why PCM, or a vendor's statement of it, is not one this wraps.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BadPcm {
    #[error("its audio's type {0} does not say its sample rate")]
    NoRate(String),
    #[error("its audio's type {0} is not one this reads")]
    Malformed(String),
    #[error("its PCM of {sample_rate} Hz, {channels} channels and {bits} bits a sample is not one this wraps")]
    Unsupported { sample_rate: u32, channels: u16, bits: u16 },
    #[error("its PCM of {bytes} bytes does not end on a whole sample of {frame} bytes")]
    Unaligned { bytes: usize, frame: usize },
    #[error("its PCM of {0} bytes is too long for a WAV file")]
    TooLong(usize),
}

/// The largest rate a WAV file of this is made at, a studio's 384 kHz.
const MAX_RATE: u32 = 384_000;
/// The most channels a WAV file of this holds, 7.1's eight.
const MAX_CHANNELS: u16 = 8;
/// A WAV header's size: `RIFF`, `fmt ` and `data` with their lengths.
const WAV_HEADER: usize = 44;

impl Pcm {
    /// PCM at `sample_rate` Hz of `channels` channels and `bits` bits a
    /// sample, when a WAV file of it plays: 1 Hz to 384 kHz, one to eight
    /// channels, 8, 16, 24 or 32 bits.
    pub fn new(sample_rate: u32, channels: u16, bits: u16) -> Result<Self, BadPcm> {
        let rate = (1..=MAX_RATE).contains(&sample_rate);
        let channel_count = (1..=MAX_CHANNELS).contains(&channels);
        let size = matches!(bits, 8 | 16 | 24 | 32);
        if !(rate && channel_count && size) {
            return Err(BadPcm::Unsupported { sample_rate, channels, bits });
        }
        Ok(Self { sample_rate, channels, bits })
    }

    /// The PCM a media type such as `audio/L16;rate=24000;channels=2`
    /// states, read as its vendor sends it: `audio/L16` (RFC 2586) and
    /// `audio/pcm` are 16-bit, at the `rate` the type must state, of the
    /// `channels` it states or one, RFC 2586's default; none for another
    /// type, a file's. A type that states no rate, or a parameter that does
    /// not read, fails rather than being guessed. Google's `audio/L16` is
    /// little-endian, as its examples write it straight into a WAV file.
    pub fn of_media_type(media_type: &str) -> Result<Option<Self>, BadPcm> {
        let mut parts = media_type.split(';').map(str::trim);
        let essence = parts.next().unwrap_or_default();
        if !essence.eq_ignore_ascii_case("audio/l16") && !essence.eq_ignore_ascii_case("audio/pcm") {
            return Ok(None);
        }
        let malformed = || BadPcm::Malformed(media_type.to_owned());
        let (mut rate, mut channels) = (None, None);
        for part in parts.filter(|part| !part.is_empty()) {
            let (name, value) = part.split_once('=').ok_or_else(malformed)?;
            let value = value.trim().trim_matches('"');
            match name.trim().to_ascii_lowercase().as_str() {
                "rate" => rate = Some(value.parse::<u32>().map_err(|_| malformed())?),
                "channels" => channels = Some(value.parse::<u16>().map_err(|_| malformed())?),
                // `codec=pcm`, as Google adds, and others say nothing of
                // the samples' shape.
                _ => {}
            }
        }
        let rate = rate.ok_or_else(|| BadPcm::NoRate(media_type.to_owned()))?;
        Self::new(rate, channels.unwrap_or(1), 16).map(Some)
    }

    pub fn sample_rate(self) -> u32 {
        self.sample_rate
    }

    pub fn channels(self) -> u16 {
        self.channels
    }

    pub fn bits(self) -> u16 {
        self.bits
    }

    /// The bytes of one sample of every channel.
    fn frame(self) -> usize {
        usize::from(self.channels) * usize::from(self.bits / 8)
    }

    /// Whether `bytes` of it are whole samples of every channel and fit a
    /// WAV file.
    pub fn check(self, bytes: usize) -> Result<(), BadPcm> {
        let frame = self.frame();
        if bytes % frame != 0 {
            return Err(BadPcm::Unaligned { bytes, frame });
        }
        u32::try_from(bytes)
            .ok()
            .and_then(|size| size.checked_add((WAV_HEADER - 8) as u32))
            .ok_or(BadPcm::TooLong(bytes))?;
        Ok(())
    }

    /// How long `bytes` of it play, in seconds.
    pub fn seconds(self, bytes: usize) -> f64 {
        bytes as f64 / (f64::from(self.sample_rate) * self.frame() as f64)
    }

    /// The WAV header of `bytes` of it, which comes before them in the
    /// file, once they are checked ([`Pcm::check`]).
    pub fn wav_header(self, bytes: usize) -> Result<[u8; WAV_HEADER], BadPcm> {
        self.check(bytes)?;
        let data = u32::try_from(bytes).map_err(|_| BadPcm::TooLong(bytes))?;
        // Within the checks of `new`: at most 384,000 × 8 × 4 bytes a
        // second and 32 bytes a frame.
        let frame = self.channels * (self.bits / 8);
        let byte_rate = self.sample_rate * u32::from(frame);
        let mut header = [0u8; WAV_HEADER];
        let mut at = 0;
        let mut put = |part: &[u8]| {
            header[at..at + part.len()].copy_from_slice(part);
            at += part.len();
        };
        put(b"RIFF");
        put(&(data + (WAV_HEADER - 8) as u32).to_le_bytes());
        put(b"WAVEfmt ");
        put(&16u32.to_le_bytes());
        // 1: integer PCM.
        put(&1u16.to_le_bytes());
        put(&self.channels.to_le_bytes());
        put(&self.sample_rate.to_le_bytes());
        put(&byte_rate.to_le_bytes());
        put(&frame.to_le_bytes());
        put(&self.bits.to_le_bytes());
        put(b"data");
        put(&data.to_le_bytes());
        Ok(header)
    }
}

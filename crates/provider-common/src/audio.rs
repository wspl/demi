//! The audio role of a provider (`audio.md` § Vendors): a provider that
//! makes audio lists its audio models, makes one file of speech, music or
//! a sound effect in one request for its key, and lists the voices a
//! speech model speaks in. Every vendor answers within the call, so a call
//! is one request, as an image's is; what every vendor's request has in
//! common is in [`crate::media`]. The backend chooses the entry.

use bytes::Bytes;
use demi_shared_types::audio::Pcm;
use demi_shared_types::{AudioKind, AudioShape, Cost, TokenUsage};
use futures_util::future::BoxFuture;

use crate::media::{MediaAnswer, MediaFailure, MediaMaker};
use crate::media_catalog::Catalog;

/// A provider's audio role.
pub trait AudioMaker: MediaMaker {
    /// The shape its requests take, which says the kinds it makes and the
    /// options it sends ([`unsent`]).
    fn shape(&self) -> AudioShape;

    /// Whether speech needs a voice named, for a vendor that has no default
    /// voice: a call without `--voice` then fails before it is sent.
    fn needs_voice(&self) -> bool {
        false
    }

    /// Makes one file of audio for the provider's key, sending with
    /// `http`, the client of the shard that asks; `catalog` says what the
    /// model makes.
    fn make<'a>(
        &'a self,
        http: &'a crate::ProviderHttp,
        catalog: &'a Catalog,
        request: AudioRequest,
    ) -> BoxFuture<'a, MediaAnswer<MadeAudio>>;

    /// The voices speech model `model` speaks in, the vendor's own and the
    /// account's, read when asked; `search`, when given, narrows them as
    /// the vendor searches. None for a vendor that lists none, whose
    /// `--voice` is sent as given.
    fn voices<'a>(
        &'a self,
        http: &'a crate::ProviderHttp,
        catalog: &'a Catalog,
        model: &'a str,
        search: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Option<Vec<Voice>>, MediaFailure>>;
}

/// One file of audio to make; an option left out is the vendor's default.
#[derive(Debug, Clone)]
pub struct AudioRequest {
    pub kind: AudioKind,
    pub model: String,
    /// The text to read aloud, or the prompt of the music or effect.
    pub text: String,
    pub voice: Option<String>,
    /// How to speak, for speech.
    pub instructions: Option<String>,
    /// A factor of the voice's speed.
    pub speed: Option<f64>,
    /// Seconds.
    pub duration: Option<f64>,
    /// An instrumental, for music.
    pub instrumental: bool,
    pub seed: Option<u64>,
    /// A sound that loops, for an effect.
    pub looped: bool,
    pub format: AudioFormatAsk,
}

/// The formats `--format` asks for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AudioFormatAsk {
    #[default]
    Mp3,
    Wav,
    Opus,
    Flac,
}

impl AudioFormatAsk {
    /// As `--format` and OpenAI's `response_format` write it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mp3 => "mp3",
            Self::Wav => "wav",
            Self::Opus => "opus",
            Self::Flac => "flac",
        }
    }
}

/// An option of `demi audio`, as a shape that has no field for it is said
/// not to take it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioOption {
    Instructions,
    Speed,
    Duration,
    Instrumental,
    Seed,
    Loop,
}

/// The options `request` gives that `shape` has no field for in a call of
/// its kind, which are not sent (`audio.md` § Commands, § Vendors), in the
/// order of the options.
pub fn unsent(shape: AudioShape, request: &AudioRequest) -> Vec<AudioOption> {
    use AudioOption::{Duration, Instructions, Instrumental, Loop, Seed, Speed};
    let lacks: &[AudioOption] = match (shape, request.kind) {
        (AudioShape::OpenaiSpeech, _) => &[],
        (AudioShape::Gemini, AudioKind::Speech) => &[Speed],
        (AudioShape::Gemini, _) => &[Duration, Instrumental, Seed, Loop],
        (AudioShape::Elevenlabs, AudioKind::Speech) => &[Instructions],
        // ElevenLabs takes no seed with a prompt, the only way a call asks.
        (AudioShape::Elevenlabs, _) => &[Seed],
    };
    let given = [
        (Instructions, request.instructions.is_some()),
        (Speed, request.speed.is_some()),
        (Duration, request.duration.is_some()),
        (Instrumental, request.instrumental),
        (Seed, request.seed.is_some()),
        (Loop, request.looped),
    ];
    given
        .into_iter()
        .filter(|(option, given)| *given && lacks.contains(option))
        .map(|(option, _)| option)
        .collect()
}

/// The audio a vendor made, as it sent it.
#[derive(Debug, Clone)]
pub struct MadeAudio {
    pub bytes: Bytes,
    /// The shape of raw PCM, for a vendor that sent no file, which the
    /// plugin wraps as WAV; none for a file.
    pub pcm: Option<Pcm>,
    /// The cost the vendor reported, in its documented unit; none
    /// otherwise.
    pub cost: Option<Cost>,
    /// The counts the vendor reported; none when it reported none.
    pub usage: Option<TokenUsage>,
    /// The vendor's id of the call, when it gives one.
    pub vendor_id: Option<String>,
}

/// A voice a speech model speaks in, as its vendor lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Voice {
    /// What `--voice` takes.
    pub id: String,
    pub name: Option<String>,
    /// The languages it speaks, as the vendor names them.
    pub languages: Vec<String>,
    /// The vendor's description, such as `Firm` or a cloned voice's.
    pub description: Option<String>,
    /// Whose it is: the vendor's built-in, or the account's own, such as
    /// a cloned or designed one; none when the vendor does not say.
    pub own: Option<bool>,
}

impl Voice {
    /// A built-in voice the vendor names alone.
    pub fn named(id: &str) -> Self {
        Self { id: id.to_owned(), name: None, languages: Vec::new(), description: None, own: Some(false) }
    }
}

/// `voices` narrowed to those `search` finds in their id, name or
/// description, ignoring case, for a vendor that searches none itself.
pub fn searched(voices: Vec<Voice>, search: Option<&str>) -> Vec<Voice> {
    let Some(search) = search.map(str::to_lowercase).filter(|search| !search.is_empty()) else {
        return voices;
    };
    voices
        .into_iter()
        .filter(|voice| {
            [Some(&voice.id), voice.name.as_ref(), voice.description.as_ref()]
                .into_iter()
                .flatten()
                .any(|text| text.to_lowercase().contains(&search))
        })
        .collect()
}

//! The `demi audio` group (`audio.md` § Commands), whose leaves the plugin
//! answers itself with the request's whole port.

use demi_host_interface::{GroupBuilder, LeafBuilder};
use demi_plugin_interface::{AudioFileFormat, CommandPlugin, Placement, PortHandled, VoiceInfo};
use demi_plugin_media::ModelsOutput;
use schemars::JsonSchema;
use serde::Deserialize;

const SUMMARY: &str = "Make speech, music and sound effects through the user's provider entries, one file per call, written to stdout, and list the voices and the audio models they offer.";

/// The group's entry in the model's capability index (`system-prompt.md`
/// § Capability index).
const ENTRY: &str = "Makes audio through the user's OpenAI, Google or ElevenLabs API key or a compatible API, with the model `--model` names: `speech` reads a text aloud in one voice, `music` makes a piece from a prompt, `effect` a sound effect; `voices` lists a speech model's voices, `models` the models with their kinds, prices and the entry a call takes. The bytes go to stdout, MP3 unless `--format` asks otherwise: `> voice.mp3` saves it; stderr says its length, format and model. Use it when the user asks for a voice-over, narration, music or a sound effect.";

const SPEECH_SUMMARY: &str = "Read the text aloud in one voice and write the audio's bytes to stdout; stderr says its length, format, model, entry and voice. An option the entry's API has no field for, a format the vendor does not make, or a result that differs from what was asked adds a warning and still succeeds.";
const MUSIC_SUMMARY: &str = "Make one piece of music from the prompt, its lyrics, structure, tempo and style written in it as the vendor documents ([Verse], [Chorus]), and write the audio's bytes to stdout; stderr says its length, format, model and entry, with a warning for each option the entry's API has no field for.";
const EFFECT_SUMMARY: &str = "Make one sound effect or ambience from the prompt and write the audio's bytes to stdout; stderr says its length, format, model and entry, with a warning for each option the entry's API has no field for.";
const VOICES_SUMMARY: &str = "List the voices a speech model speaks in, as the entry a call of it takes lists them: the vendor's built-in voices and the user's own, such as a cloned one, each with its ID, name, languages and the vendor's description. A read.";
const MODELS_SUMMARY: &str = "List the audio models the user's provider entries offer, newest first, with their kinds and the entry a call takes now, or show one model with everything known of it: kinds, voices, languages, instructions, lengths, lyrics, instrumentals, loops, formats, prices per unit and currency, aliases, and the entries in the order a call tries them with why each would or would not be used now. A read.";

const TEXT: &str = "The text to read aloud; - reads it from stdin.";
const PROMPT: &str = "What to make; - reads it from stdin.";
const MODEL: &str = "The audio model, required: one `demi audio models` lists that makes this kind.";
const PROVIDER: &str = "The entry to make it through, by its name in Settings or its id, when the user asked for one; without it, the first that can make the model now, in the order of Settings > Providers.";
const VOICE: &str = "A voice `demi audio voices --model <id>` lists; the vendor's default without it, and ElevenLabs, which has none, needs it.";
const INSTRUCTIONS: &str = "How to speak, such as a tone, a pace or an accent, for the models that take it.";
const SPEED: &str = "A factor of the voice's speed, such as 1.2.";
const SPEECH_FORMAT: &str = "The format to ask for; MP3 without it. A vendor that makes another says so on stderr; nothing is transcoded.";
const CLIP_FORMAT: &str = "The format to ask for; MP3 without it.";
const DURATION: &str = "The length in seconds; the model's choice without it.";
const INSTRUMENTAL: &str = "Ask for music without vocals.";
const SEED: &str = "A seed, for the vendors that take one.";
const LOOP: &str = "Ask for a sound that loops smoothly.";
const VOICES_MODEL: &str = "The speech model whose voices to list, required.";
const SEARCH: &str = "Words to narrow the voices by, in their ID, name or description.";
const SHOWN: &str = "The model to show; without it, every model is listed.";

const SUCCESS: &str = "the audio's bytes on stdout; on stderr a line such as `0:04 MP3 speech from gpt-4o-mini-tts (OpenAI), voice coral` or `0:15 MP3 music from music_v2 (ElevenLabs)`, with the cost where the vendor reports one, and a warning for each option the entry's API takes no field for, a format the vendor does not make, or a length more than a second from --duration";

const FAILURE: &str = "no --model, no provider that makes audio, a model none makes or that makes another kind (with the kinds it makes), an entry --provider names that cannot make it now (with why and the entries that can), every entry that could used up, a refused API key, a refusal in the vendor's words, ElevenLabs speech without --voice, a vendor that did not answer in 120 s, or a file that is not audio this reads, each named on stderr with exit status 1";

/// The formats music and effects are asked in.
#[derive(Clone, Copy, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ClipFormat {
    #[default]
    Mp3,
    Wav,
}

impl From<ClipFormat> for AudioFileFormat {
    fn from(format: ClipFormat) -> Self {
        match format {
            ClipFormat::Mp3 => Self::Mp3,
            ClipFormat::Wav => Self::Wav,
        }
    }
}

/// The input of `speech`.
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) struct SpeechArgs {
    pub(crate) text: String,
    pub(crate) model: Option<String>,
    pub(crate) provider: Option<String>,
    pub(crate) voice: Option<String>,
    pub(crate) instructions: Option<String>,
    pub(crate) speed: Option<f64>,
    pub(crate) format: Option<AudioFileFormat>,
}

/// The input of `music`.
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) struct MusicArgs {
    pub(crate) prompt: String,
    pub(crate) model: Option<String>,
    pub(crate) provider: Option<String>,
    pub(crate) duration: Option<f64>,
    pub(crate) instrumental: Option<bool>,
    pub(crate) seed: Option<u64>,
    pub(crate) format: Option<ClipFormat>,
}

/// The input of `effect`.
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) struct EffectArgs {
    pub(crate) prompt: String,
    pub(crate) model: Option<String>,
    pub(crate) provider: Option<String>,
    pub(crate) duration: Option<f64>,
    #[serde(rename = "loop")]
    pub(crate) looped: Option<bool>,
    pub(crate) format: Option<ClipFormat>,
}

/// The input of `voices`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct VoicesArgs {
    pub(crate) model: Option<String>,
    pub(crate) search: Option<String>,
}

/// The input of `models`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModelsArgs {
    pub(crate) model: Option<String>,
}

/// The plugin's commands: the `demi audio` group.
pub(crate) fn commands() -> CommandPlugin {
    CommandPlugin::new(Placement::Demi, vec![audio_group()]).expect("the audio group is a valid declaration")
}

fn audio_group() -> GroupBuilder {
    GroupBuilder::new("audio", SUMMARY)
        .index_entry(ENTRY)
        .leaf(
            LeafBuilder::rpc("speech", SPEECH_SUMMARY)
                .input::<SpeechArgs>()
                .describe("text", TEXT)
                .describe("model", MODEL)
                .describe("provider", PROVIDER)
                .describe("voice", VOICE)
                .describe("instructions", INSTRUCTIONS)
                .describe("speed", SPEED)
                .describe("format", SPEECH_FORMAT)
                .positionals(["text"])
                .success_output(SUCCESS)
                .failure_output(FAILURE)
                .bind(PortHandled),
        )
        .leaf(
            LeafBuilder::rpc("music", MUSIC_SUMMARY)
                .input::<MusicArgs>()
                .describe("prompt", PROMPT)
                .describe("model", MODEL)
                .describe("provider", PROVIDER)
                .describe("duration", DURATION)
                .describe("instrumental", INSTRUMENTAL)
                .describe("seed", SEED)
                .describe("format", CLIP_FORMAT)
                .positionals(["prompt"])
                .success_output(SUCCESS)
                .failure_output(FAILURE)
                .bind(PortHandled),
        )
        .leaf(
            LeafBuilder::rpc("effect", EFFECT_SUMMARY)
                .input::<EffectArgs>()
                .describe("prompt", PROMPT)
                .describe("model", MODEL)
                .describe("provider", PROVIDER)
                .describe("duration", DURATION)
                .describe("loop", LOOP)
                .describe("format", CLIP_FORMAT)
                .positionals(["prompt"])
                .success_output(SUCCESS)
                .failure_output(FAILURE)
                .bind(PortHandled),
        )
        .leaf(
            LeafBuilder::rpc("voices", VOICES_SUMMARY)
                .input::<VoicesArgs>()
                .describe("model", VOICES_MODEL)
                .describe("search", SEARCH)
                .positionals(["search"])
                .json_output::<Vec<VoiceInfo>>()
                .bind(PortHandled),
        )
        .leaf(
            LeafBuilder::rpc("models", MODELS_SUMMARY)
                .input::<ModelsArgs>()
                .describe("model", SHOWN)
                .positionals(["model"])
                .json_output::<ModelsOutput>()
                .bind(PortHandled),
        )
}

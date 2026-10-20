//! The `demi video` group, whose leaves the plugin answers itself with
//! the request's whole port.

use demi_host_interface::{GroupBuilder, LeafBuilder};
use demi_plugin_interface::{CommandPlugin, Placement, PortHandled, VideoResolution};
use demi_plugin_media::ModelsOutput;
use schemars::JsonSchema;
use serde::Deserialize;

const SUMMARY: &str = "Make videos through the user's provider entries, one MP4 per call, written to stdout, and list the video models they offer.";

/// The group's entry in the model's capability index.
const ENTRY: &str = "Makes videos through the user's Grok subscription, xAI, Google, MiniMax, BytePlus ModelArk or Volcengine Ark API key, or a compatible API: `generate` makes one MP4 from a prompt with the model `--model` names, optionally from a first frame, a last frame and reference images of the Host; `models` lists the models with their durations, resolutions, prices per second and the entry a call takes. The call waits for the vendor's job, minutes at most, saying on stderr how it moves; the video's bytes go to stdout: `> clip.mp4` saves it. Use it when the user asks for a video, a clip or an animation.";

const GENERATE_SUMMARY: &str = "Make one video from the prompt, waiting for the vendor's job, and write its MP4 to stdout; stderr names the job at once, says each change of its state, then the video's length, size, sound, model, entry and cost. An option the result does not honour, or the entry's API has no field for, adds a warning and still succeeds.";

const MODELS_SUMMARY: &str = "List the video models the user's provider entries offer, newest first, with the entry a call takes now, or show one model with everything known of it: durations, aspect ratios, resolutions, frames, reference images, sound, prices per unit and currency, aliases, and the entries in the order a call tries them with why each would or would not be used now. PRICE is included for a subscription and listed for an API key, which pays for each second as Prices says. A read.";

const PROMPT: &str = "What the video shows and how it moves; - reads it from stdin.";
const MODEL: &str = "The video model, required: one `demi video models` lists.";
const PROVIDER: &str = "The entry to make it through, by its name in Settings or its id, when the user asked for one; without it, the first that can make the model now, subscriptions before API keys, each in the order of Settings > Providers.";
const IMAGE: &str = "The first frame: a PNG, JPEG or WebP file of this Host of at most 16 MiB, relative to the current directory or absolute.";
const LAST_FRAME: &str = "The last frame, which needs --image; a file as --image takes.";
const REFERENCE: &str = "An image the video keeps to, such as a character or a product; repeat it for several. A file as --image takes.";
const DURATION: &str = "The length in seconds; the model's default without it.";
const ASPECT: &str = "The shape to ask for, as W:H, such as 16:9.";
const RESOLUTION: &str = "The resolution to ask for: a height such as 720p or 768p, or 2k or 4k, any the vendor names; `demi video models` lists each model's.";
const AUDIO: &str = "Ask for the sound the model makes.";
const NO_AUDIO: &str = "Ask for a silent video.";
const SEED: &str = "A seed, for the vendors that take one.";
const SHOWN: &str = "The model to show; without it, every model is listed.";

const SUCCESS: &str = "the MP4's bytes on stdout; on stderr the job asked for, such as `asked grok-imagine-video-1.5 (Grok, alex@example.com), job d97415a1-5796-b7ec-379f-4e6819e08fdf`, each change of its state, `queued`, `running` or `running, 40%`, and a line such as `5 s 1280×720 MP4 with sound from grok-imagine-video-1.5 (Grok, alex@example.com), $0.25`, with a warning for each option the result does not honour or the entry's API takes no field for";

const FAILURE: &str = "no --model, no provider that makes videos, a model none makes, an entry --provider names that cannot make it now (with why and the entries that can), every entry that could used up, a refused API key, a refusal, an expired sign-in, a vendor that did not answer a request in 120 s, a job the vendor failed or that expired there, a job still running after 10 minutes (with its id), a file that is not an MP4, or an image that is not a PNG, JPEG or WebP file of at most 16 MiB, each named on stderr with exit status 1";

/// The input of `generate`.
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) struct GenerateArgs {
    pub(crate) prompt: String,
    pub(crate) model: Option<String>,
    pub(crate) provider: Option<String>,
    pub(crate) image: Option<String>,
    pub(crate) last_frame: Option<String>,
    pub(crate) reference: Option<Vec<String>>,
    #[schemars(range(min = 1))]
    pub(crate) duration: Option<u32>,
    pub(crate) aspect: Option<String>,
    pub(crate) resolution: Option<VideoResolution>,
    pub(crate) audio: Option<bool>,
    pub(crate) no_audio: Option<bool>,
    pub(crate) seed: Option<u64>,
}

/// The input of `models`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModelsArgs {
    pub(crate) model: Option<String>,
}

/// The plugin's commands: the `demi video` group.
pub(crate) fn commands() -> CommandPlugin {
    CommandPlugin::new(Placement::Demi, vec![video_group()]).expect("the video group is a valid declaration")
}

fn video_group() -> GroupBuilder {
    GroupBuilder::new("video", SUMMARY)
        .index_entry(ENTRY)
        .leaf(
            LeafBuilder::rpc("generate", GENERATE_SUMMARY)
                .input::<GenerateArgs>()
                .describe("prompt", PROMPT)
                .describe("model", MODEL)
                .describe("provider", PROVIDER)
                .describe("image", IMAGE)
                .describe("last-frame", LAST_FRAME)
                .describe("reference", REFERENCE)
                .describe("duration", DURATION)
                .describe("aspect", ASPECT)
                .describe("resolution", RESOLUTION)
                .describe("audio", AUDIO)
                .describe("no-audio", NO_AUDIO)
                .describe("seed", SEED)
                .positionals(["prompt"])
                .success_output(SUCCESS)
                .failure_output(FAILURE)
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

//! The `demi image` group (`images.md` § Commands), whose leaves the
//! plugin answers itself with the request's whole port.

use demi_host_interface::{GroupBuilder, LeafBuilder};
use demi_plugin_interface::{CommandPlugin, Placement, PortHandled};
use demi_plugin_media::ModelsOutput;
use schemars::JsonSchema;
use serde::Deserialize;

const SUMMARY: &str = "Make and edit images through the user's provider entries, one image per call, written to stdout, and list the image models they offer.";

/// The group's entry in the model's capability index (`system-prompt.md`
/// § Capability index).
const ENTRY: &str = "Makes and edits images through the user's ChatGPT or Grok subscription, OpenAI, xAI, Google, MiniMax, BytePlus ModelArk or Volcengine Ark API key, or a compatible API: `generate` makes one from a prompt with the model `--model` names, `edit` from a prompt and images of the Host; `models` lists the models with their sizes, prices and the entry a call takes. The image's bytes go to stdout: `> icon.png` saves it, `| demi file view` shows it to you; stderr says what came back. Several images are several calls. Use it when the user asks for an image, an icon or an illustration, or to change one.";

const GENERATE_SUMMARY: &str = "Make one image from the prompt and write its bytes to stdout; stderr says its size, format, model and entry. An option the result does not honour adds a warning and still succeeds.";

const EDIT_SUMMARY: &str = "Make one image from the prompt and the images given, each changed, combined or used as a reference as the prompt says, and write its bytes to stdout. Each image is a PNG, JPEG or WebP file of this Host of at most 16 MiB.";

const MODELS_SUMMARY: &str = "List the image models the user's provider entries offer, newest first, with the entry a call takes now, or show one model with everything known of it: sizes, edits, input images, transparency, prices per unit and currency, aliases, and the entries in the order a call tries them with why each would or would not be used now. PRICE is included for a subscription, whose images come with its plan, and listed for an API key, which pays for each as Prices says. A read.";

const PROMPT: &str = "What to make, or how to change the images; - reads it from stdin.";
const IMAGE: &str = "The images to edit, relative to the current directory or absolute.";
const MODEL: &str = "The image model, required: one `demi image models` lists.";
const PROVIDER: &str = "The entry to make it through, by its name in Settings or its id, when the user asked for one; without it, the first that can make the model now, subscriptions before API keys, each in the order of Settings > Providers.";
const ASPECT: &str = "The shape to ask for, as W:H, such as 16:9.";
const TRANSPARENT: &str = "Ask for a transparent background, which needs PNG or WebP.";
const FORMAT: &str = "The format of the bytes written, whatever the vendor sent; PNG without it.";
const SHOWN: &str = "The model to show; without it, every model is listed.";

const SUCCESS: &str = "the image's bytes on stdout; on stderr a line such as `864×1152 PNG from grok-imagine-image (Grok, alex@example.com)`, or `1254×1254 PNG, asked of gpt-image-2 (ChatGPT, alex@example.com)` from a vendor that does not confirm the model, and a warning for each option the result does not honour";

const FAILURE: &str = "no --model, no provider that makes images, a model none makes, an entry --provider names that cannot make it now (with why and the entries that can), every entry that could used up, a refused API key, a refusal, a shape the vendor does not take (with the shapes it takes), an expired sign-in or a vendor that did not answer in 120 s, each named on stderr with exit status 1";

/// The format of the bytes `generate` and `edit` write.
#[derive(Clone, Copy, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Format {
    #[default]
    Png,
    Jpeg,
    Webp,
}

/// The input of `generate`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct GenerateArgs {
    pub(crate) prompt: String,
    pub(crate) model: Option<String>,
    pub(crate) provider: Option<String>,
    pub(crate) aspect: Option<String>,
    pub(crate) transparent: Option<bool>,
    pub(crate) format: Option<Format>,
}

/// The input of `edit`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct EditArgs {
    pub(crate) prompt: String,
    #[schemars(length(min = 1))]
    pub(crate) image: Vec<String>,
    pub(crate) model: Option<String>,
    pub(crate) provider: Option<String>,
    pub(crate) aspect: Option<String>,
    pub(crate) transparent: Option<bool>,
    pub(crate) format: Option<Format>,
}

/// The input of `models`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModelsArgs {
    pub(crate) model: Option<String>,
}

/// The plugin's commands: the `demi image` group.
pub(crate) fn commands() -> CommandPlugin {
    CommandPlugin::new(Placement::Demi, vec![image_group()])
        .expect("the image group is a valid declaration")
}

fn image_group() -> GroupBuilder {
    GroupBuilder::new("image", SUMMARY)
        .index_entry(ENTRY)
        .leaf(
            LeafBuilder::rpc("generate", GENERATE_SUMMARY)
                .input::<GenerateArgs>()
                .describe("prompt", PROMPT)
                .describe("model", MODEL)
                .describe("provider", PROVIDER)
                .describe("aspect", ASPECT)
                .describe("transparent", TRANSPARENT)
                .describe("format", FORMAT)
                .positionals(["prompt"])
                .success_output(SUCCESS)
                .failure_output(FAILURE)
                .bind(PortHandled),
        )
        .leaf(
            LeafBuilder::rpc("edit", EDIT_SUMMARY)
                .input::<EditArgs>()
                .describe("prompt", PROMPT)
                .describe("image", IMAGE)
                .describe("model", MODEL)
                .describe("provider", PROVIDER)
                .describe("aspect", ASPECT)
                .describe("transparent", TRANSPARENT)
                .describe("format", FORMAT)
                .positionals(["prompt", "image"])
                .success_output(SUCCESS)
                .failure_output(format!("{FAILURE}; an image that is not a PNG, JPEG or WebP file, or is over 16 MiB, named the same way"))
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

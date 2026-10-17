//! The `demi image` group (`images.md` § Commands), whose leaves the
//! plugin answers itself with the request's whole port.

use demi_host_interface::{GroupBuilder, LeafBuilder};
use demi_plugin_interface::{CommandPlugin, Placement, PortHandled};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const SUMMARY: &str = "Make and edit images through the user's ChatGPT or Grok subscription, one image per call, written to stdout.";

/// The group's entry in the model's capability index (`system-prompt.md`
/// § Capability index).
const ENTRY: &str = "Makes and edits images through the user's ChatGPT or Grok subscription: `generate` makes one from a prompt, `edit` from a prompt and images of the Host. The image's bytes go to stdout: `> icon.png` saves it, `| demi file view` shows it to you; stderr says what came back. Several images are several calls. Use it when the user asks for an image, an icon or an illustration, or to change one.";

const GENERATE_SUMMARY: &str = "Make one image from the prompt and write its bytes to stdout; stderr says its size, format, model and account. An option the result does not honour adds a warning and still succeeds.";

const EDIT_SUMMARY: &str = "Make one image from the prompt and the images given, each changed, combined or used as a reference as the prompt says, and write its bytes to stdout. Each image is a PNG, JPEG or WebP file of this Host of at most 16 MiB.";

const MODELS_SUMMARY: &str = "List the image models of the user's provider entries, one per line: its entry's name, its release date, newest first, the default marked, what its vendor says it is for and the sizes or shapes it takes; a provider that cannot confirm which model answered says so under its models. A read.";

const PROMPT: &str = "What to make, or how to change the images; - reads it from stdin.";
const IMAGE: &str = "The images to edit, relative to the current directory or absolute.";
const MODEL: &str = "An image model `demi image models` lists, which picks its entry; the first entry's default model without it.";
const ASPECT: &str = "The shape to ask for, as W:H, such as 16:9.";
const TRANSPARENT: &str = "Ask for a transparent background, which needs PNG or WebP.";
const FORMAT: &str = "The format of the bytes written, whatever the vendor sent; PNG without it.";

const SUCCESS: &str = "the image's bytes on stdout; on stderr a line such as `864×1152 PNG from grok-imagine-image (Grok, alex@example.com)`, or `1254×1254 PNG, asked of gpt-image-2 (ChatGPT, alex@example.com)` from a vendor that does not confirm the model, and a warning for each option the result does not honour";

const FAILURE: &str = "no provider that makes images, a model none makes, every account used up, a refusal, a shape the vendor does not take (with the shapes it takes), an expired sign-in or a vendor that did not answer in 120 s, each named on stderr with exit status 1";

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
    pub(crate) aspect: Option<String>,
    pub(crate) transparent: Option<bool>,
    pub(crate) format: Option<Format>,
}

/// The input of `models`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModelsArgs {}

/// `models --json`.
#[derive(Serialize, JsonSchema)]
pub(crate) struct Listing {
    pub(crate) models: Vec<ListedModel>,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ListedModel {
    pub(crate) id: String,
    /// The entry's name as Settings shows it.
    pub(crate) provider: String,
    pub(crate) provider_id: String,
    /// The vendor's release date, `YYYY-MM-DD`.
    pub(crate) released: String,
    /// What the vendor publishes the model is for.
    pub(crate) description: String,
    /// The sizes or shapes it takes, and what the vendor says of its
    /// formats.
    pub(crate) takes: String,
    /// Whether the vendor confirms the model that answered; a call to one
    /// that does not says `asked of` the model.
    pub(crate) confirmed: bool,
    /// Whether a call without --model takes it.
    pub(crate) default: bool,
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
                .json_output::<Listing>()
                .bind(PortHandled),
        )
}

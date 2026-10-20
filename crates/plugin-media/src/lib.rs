//! What the media plugins share, as a library: the prompt read from
//! stdin, the input images read through the port's file reads, the words
//! of a media refusal for the plugin's kind, and the `models` command's
//! listing and one-model view on the shared renderer.
//!
//! ```text
//! plugin-image  plugin-video  plugin-audio    help, option checks, bytes out
//!        \            |            /
//!         Providers service of the port       one call: kind, model, entry?
//!                     |
//!         backend (shard/media.rs)            entry, account, hand-on, ledger
//!                     |
//!         provider-* crates                   the vendors' APIs
//! ```
//!
//! There are three plugins, one per group, so the user can turn one kind off,
//! as videos cost the most, and the agent then has no command for it. A
//! plugin holds only its commands: their help, the checks of their options and
//! the bytes they write. One backend service makes every kind, so choosing an
//! entry, taking an account, handing on when one is used up, the usage ledger
//! and the catalog are written once.
//!
//! The agent names the model, always: no media comes from a model nobody
//! chose. Which entry makes it is the user's setting, the order of Settings ›
//! Providers and each entry's switch, plus `--provider` when the user asked
//! for one entry; they say everything a setting per vendor or per model
//! would, for every kind at once. `models` is the agent's whole view, so it
//! can choose without searching the web or trying calls. Every option goes to
//! the vendor as asked, even where the vendor is known to ignore it, in the
//! field its API has for it, and the result is compared with what was asked:
//! a vendor that starts to honour an option needs no change in Demi, and a
//! result that differs adds a warning and still succeeds. The bytes go to
//! stdout, one file per call.

mod input;
mod models;
mod words;

pub use input::{MAX_IMAGE_BYTES, decode_args, fits, parse_aspect, prompt, read_images, required_model};
pub use models::{ListedModel, ModelsOutput, models};
pub use words::{listed, refusal_words, refused};

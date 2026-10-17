//! What the media plugins share (`media.md`), as a library: the prompt
//! read from stdin, the input images read through the port's file reads,
//! the words of a media refusal for the plugin's kind, and the `models`
//! command's listing and one-model view on the shared renderer.

mod input;
mod models;
mod words;

pub use input::{MAX_IMAGE_BYTES, decode_args, fits, parse_aspect, prompt, read_images, required_model};
pub use models::{ListedModel, ModelsOutput, models};
pub use words::{listed, refusal_words, refused};

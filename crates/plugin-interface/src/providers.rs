//! The port's Providers service (`plugins.md` § The contract, `images.md`):
//! the image models of the user's provider entries, and one image made
//! through an entry's account. The backend chooses the entry and the
//! account, rotates them, renews their sign-in and records the call in the
//! usage ledger; no token reaches the plugin.

use demi_shared_types::{B64Bytes, Timestamp};
use serde::{Deserialize, Serialize};

/// An image model of one of the user's entries; the list is the entries in
/// the order of Settings › Providers, each entry's models newest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImageModel {
    pub id: String,
    /// The entry's name, as Settings › Providers shows it.
    pub provider: String,
    pub provider_id: String,
    /// How the user knows the vendor, such as `ChatGPT`.
    pub vendor: String,
    /// The vendor's release date, `YYYY-MM-DD`.
    pub released: String,
    /// What the vendor publishes the model is for, in one line.
    pub description: String,
    /// The sizes or shapes it takes, and what the vendor says of its
    /// formats.
    pub takes: String,
    /// Whether a call that names no model takes it.
    pub default: bool,
    /// Whether the entry's vendor confirms the model that answered; one
    /// that does not, ChatGPT, is shown as asked (`images.md` § Commands).
    pub confirmed: bool,
}

/// One image to make: from the prompt alone, or from the prompt and the
/// images given.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImageAsk {
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<ImageInput>,
    /// The shape asked for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aspect: Option<ImageAspect>,
    pub transparent: bool,
    /// An image model of [`ImageModel`]'s list, which picks its entry; the
    /// first entry's first model without it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// A shape, such as 16:9: two positive whole numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImageAspect {
    pub width: u32,
    pub height: u32,
}

/// An image an edit sends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImageInput {
    pub kind: ImageKind,
    pub bytes: B64Bytes,
}

/// The kinds of image an edit sends, by their media types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImageKind {
    #[serde(rename = "image/png")]
    Png,
    #[serde(rename = "image/jpeg")]
    Jpeg,
    #[serde(rename = "image/webp")]
    Webp,
}

impl ImageKind {
    pub fn media_type(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Webp => "image/webp",
        }
    }
}

/// The image made, with the entry, account and model that made it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MadeImage {
    /// As the vendor answered it; the plugin reads what it is.
    pub bytes: B64Bytes,
    pub model: String,
    pub provider: String,
    pub vendor: String,
    /// The account's label, such as its email, for a user who manages the
    /// entry; none for a user of a shared instance's entries, whose
    /// account is the administrator's (`providers.md` § Scope).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    /// Whether the vendor confirms `model` made the image; otherwise it is
    /// only the model asked for.
    pub confirmed: bool,
}

/// Why no image was made (`images.md` § Accounts and failures).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ImageRefusal {
    /// No entry the user has on makes images.
    #[error("no provider makes images")]
    NoProvider,
    /// No entry makes the model asked for.
    #[error("no provider makes \"{model}\"")]
    UnknownModel { model: String },
    /// Every entry that could make images has its Image Generation off;
    /// it names them, as Settings › Providers does.
    #[error("image generation is off for {}", providers.join(", "))]
    AllOff { providers: Vec<String> },
    /// The model asked for is made by an entry whose Image Generation is
    /// off, which it names.
    #[error("\"{model}\" is {provider}'s, whose image generation is off")]
    Off { model: String, provider: String },
    /// Every account the call could take is used up, until the moment
    /// named, when known.
    #[error("{vendor}'s usage limit is reached")]
    UsedUp {
        vendor: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resets_at: Option<Timestamp>,
    },
    /// The vendor refused the prompt or an image, in its own words.
    #[error("{vendor} refused the request: {message}")]
    Refused { vendor: String, message: String },
    /// The vendor refused the shape asked for, naming the shapes it takes.
    #[error("{vendor} takes none of the shape asked for")]
    Shape { vendor: String, takes: Vec<String> },
    /// The account's sign-in expired and cannot be renewed; the account is
    /// named as [`MadeImage`] names it.
    #[error("the {vendor} sign-in has expired")]
    Expired {
        vendor: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        account: Option<String>,
    },
    /// The vendor failed, or did not answer in time.
    #[error("{vendor} did not answer: {cause}")]
    Unanswered { vendor: String, cause: String },
    /// The backend could not make the call, such as a store that failed.
    #[error("{message}")]
    Failed { message: String },
}

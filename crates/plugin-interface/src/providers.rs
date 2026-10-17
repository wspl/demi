//! The port's Providers service (`plugins.md` § The contract, `images.md`):
//! the image models of the user's provider entries, by vendor, and one
//! image made through an entry's account or key. The backend chooses the
//! entry and the account, rotates them, renews their sign-in and records
//! the call in the usage ledger; no token or key reaches the plugin.

use demi_shared_types::{B64Bytes, Timestamp};
use serde::{Deserialize, Serialize};

/// The image models of one vendor's entries (`images.md` § Commands): the
/// groups of `demi image models`, OpenAI's, Google's and xAI's in that
/// order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImageVendorModels {
    /// How the user knows the vendor, such as `OpenAI`.
    pub vendor: String,
    /// Its models.dev id, such as `openai`.
    pub vendor_id: String,
    /// The entry its models go through now: the one the user chose, or the
    /// one Automatic takes (`images.md` § Which entry makes an image).
    pub through: ImageEntry,
    /// Newest first by release date, those without one last.
    pub models: Vec<ImageModel>,
    /// The entries whose models could not be listed, each with why.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unlisted: Vec<ImageUnlisted>,
}

/// An entry an image is made through.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImageEntry {
    /// The entry's name, as Settings › Providers shows it.
    pub provider: String,
    pub provider_id: String,
    /// The account's label for a user who manages the entry, as
    /// [`MadeImage`] names it; none for a key or a shared instance's user.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    /// Whether the entry's vendor confirms the model that answered; one
    /// that does not, ChatGPT, is shown as asked (`images.md` § Commands).
    pub confirmed: bool,
}

/// An image model of a vendor's entries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImageModel {
    pub id: String,
    /// The vendor's release date, `YYYY-MM-DD`, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub released: Option<String>,
    /// What the vendor says the model is for; empty when it says nothing.
    pub description: String,
    /// The sizes or shapes it takes, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub takes: Option<String>,
    /// The entry it goes through when that is not the group's: a model
    /// only another entry of the vendor offers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub through: Option<ImageEntry>,
}

/// An entry whose models could not be listed, and why, as a call's
/// refusal says it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImageUnlisted {
    pub provider: String,
    pub refusal: ImageRefusal,
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
    /// An image model `demi image models` lists, which names its vendor;
    /// the user's setting picks the entry.
    pub model: String,
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
    /// Every entry that could make the model is used up, the first named
    /// with the moment it can be used again, when known.
    #[error("{vendor}'s usage limit is reached")]
    UsedUp {
        vendor: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resets_at: Option<Timestamp>,
    },
    /// The vendor refused the entry's API key; `manages` says whether the
    /// user manages the entry, and so can check it.
    #[error("{vendor} refused the API key")]
    KeyRefused { vendor: String, manages: bool },
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

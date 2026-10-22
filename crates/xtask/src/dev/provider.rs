//! The real model `xtask dev` can seed, from the `DEMI_DEV_PROVIDER_*`
//! variables: all four name one model of an OpenAI-compatible Chat
//! Completions endpoint, or none is set, and two optional ones list the
//! model's thinking efforts and the file types it reads natively.

use std::num::NonZeroU32;

use demi_shared_types::{ATTACHMENT_FILE_EXTENSIONS, FileExtension, VIDEO_FILE_EXTENSIONS};
use serde_json::json;

/// The variables, in the order the entry needs them.
const NAMES: [&str; 4] = [
    "DEMI_DEV_PROVIDER_BASE_URL",
    "DEMI_DEV_PROVIDER_API_KEY",
    "DEMI_DEV_PROVIDER_MODEL",
    "DEMI_DEV_PROVIDER_CONTEXT_WINDOW",
];
/// The model's thinking efforts, comma-separated, as the endpoint names
/// them; without it the model has none, and the page offers no effort.
const EFFORTS: &str = "DEMI_DEV_PROVIDER_THINKING_EFFORTS";
/// The file types the model reads natively, comma-separated, as its
/// `acceptedExtensions` names them (`crates/shared-types/src/model.rs`);
/// without it which types the model reads is unknown.
const ACCEPTED: &str = "DEMI_DEV_PROVIDER_ACCEPTED_EXTENSIONS";

/// The development model the variables name.
pub struct DevProvider {
    base_url: String,
    api_key: String,
    model: String,
    context_window: NonZeroU32,
    thinking_efforts: Vec<String>,
    accepted_extensions: Option<Vec<FileExtension>>,
}

impl DevProvider {
    /// The model `var` names, none when it names none of the variables, or
    /// what is wrong with them. Read before anything is built, so a partial
    /// setting stops the command at once; the backend validates the URL when
    /// the entry is created.
    pub fn read(var: impl Fn(&str) -> Option<String>) -> Result<Option<Self>, String> {
        let values = NAMES.map(|name| var(name).filter(|value| !value.is_empty()));
        let efforts = var(EFFORTS).filter(|value| !value.is_empty());
        let accepted = var(ACCEPTED).filter(|value| !value.is_empty());
        if values.iter().all(Option::is_none) {
            let optional = [(EFFORTS, &efforts), (ACCEPTED, &accepted)];
            return match optional.iter().find(|(_, value)| value.is_some()) {
                Some((name, _)) => Err(format!(
                    "{name} is set, but the development provider needs all of {}",
                    NAMES.join(", ")
                )),
                None => Ok(None),
            };
        }
        let [Some(base_url), Some(api_key), Some(model), Some(window)] = values else {
            let missing: Vec<&str> = NAMES
                .iter()
                .zip(&values)
                .filter(|(_, value)| value.is_none())
                .map(|(name, _)| *name)
                .collect();
            return Err(format!(
                "{} not set; the development provider needs all of {}",
                missing.join(", "),
                NAMES.join(", ")
            ));
        };
        let context_window = window
            .parse::<NonZeroU32>()
            .map_err(|_| format!("{} must be a positive integer below 2^32", NAMES[3]))?;
        let thinking_efforts = efforts
            .iter()
            .flat_map(|list| list.split(','))
            .map(|effort| effort.trim().to_owned())
            .collect();
        let accepted_extensions = accepted
            .map(|list| list.split(',').map(accepted_extension).collect())
            .transpose()?;
        Ok(Some(Self {
            base_url,
            api_key,
            model,
            context_window,
            thinking_efforts,
            accepted_extensions,
        }))
    }

    /// The model's name, as the page shows it.
    pub fn model(&self) -> &str {
        &self.model
    }

    /// The file types the model reads natively, none when that is unknown.
    pub fn accepted_extensions(&self) -> Option<&[FileExtension]> {
        self.accepted_extensions.as_deref()
    }

    /// The provider entry the web API creates: an `openai` entry labeled
    /// Development that speaks Chat Completions, with the one model.
    pub fn entry(&self) -> serde_json::Value {
        json!({
            "source": "compatible",
            "providerType": "openai",
            "wireApi": "chat-completions",
            "label": "Development",
            "apiKey": self.api_key,
            "baseUrl": self.base_url,
            "models": [{
                "id": self.model,
                "displayName": self.model,
                "contextWindow": self.context_window.get(),
                "outputLimit": null,
                "thinkingEfforts": self.thinking_efforts,
                "acceptedExtensions": self.accepted_extensions,
                "fastTier": null,
            }],
        })
    }
}

/// The model-media type `name` names, or what the command says when it
/// names none.
fn accepted_extension(name: &str) -> Result<FileExtension, String> {
    let name = name.trim();
    name.parse().map_err(|_| {
        let known: Vec<String> = ATTACHMENT_FILE_EXTENSIONS
            .iter()
            .chain(&VIDEO_FILE_EXTENSIONS)
            .map(ToString::to_string)
            .collect();
        format!(
            "{ACCEPTED} names \"{name}\", which no model reads natively; the types are {}",
            known.join(", ")
        )
    })
}

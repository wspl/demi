//! A job of the Interactions API in the background, which the key's video,
//! image, speech and music roles share
//! ([background execution](https://ai.google.dev/gemini-api/docs/background-execution)):
//!
//! 1. `POST …/interactions` with `background: true` and `store: true`
//!    answers the interaction at once, its `id` the job's.
//! 2. `GET …/interactions/<id>` reads it: `queued` or `in_progress` until it
//!    ends `completed`, with what it made in its `model_output` steps, or
//!    `failed`, `cancelled` or `incomplete`, with its errors.
//!
//! Nothing but the id joins the two, so a server that restarted reads the job
//! again. Each role's body says what it asks; this module asks and reads.

use bytes::Bytes;
use demi_provider_common::{
    endpoint_url,
    media::{MediaAnswer, MediaFailure, unreadable},
};
use demi_shared_types::{B64Bytes, TokenUsage, VendorTrace};
use serde::Deserialize;

use crate::images::GeminiKey;

/// Asks for the interaction `body` describes, in the background, and
/// answers its id. `body` is a JSON object without `background` and
/// `store`, which are set here.
pub(crate) async fn create(key: &GeminiKey, http: &demi_provider_common::ProviderHttp, mut body: serde_json::Value) -> MediaAnswer<String> {
    if let Some(object) = body.as_object_mut() {
        object.insert("background".to_owned(), serde_json::Value::Bool(true));
        object.insert("store".to_owned(), serde_json::Value::Bool(true));
    }
    let url = endpoint_url(key.base(), "/interactions");
    let headers = key.headers();
    key.call()
        .post_json(http, &url, Bytes::from(demi_provider_common::json_body(&body)), |_| {
            std::future::ready(Ok((headers.clone(), None)))
        })
        .await
        .and_then(|answer| {
            let created: Interaction = read(&answer)?;
            created.id.ok_or_else(|| unreadable("its created interaction named no id".to_owned()))
        })
}

/// Reads the interaction `id` once.
pub(crate) async fn get(key: &GeminiKey, http: &demi_provider_common::ProviderHttp, id: &str) -> MediaAnswer<Interaction> {
    let url = endpoint_url(key.base(), &format!("/interactions/{id}"));
    let result = async { read(&key.call().get(http, &url, key.headers()).await?) }.await;
    MediaAnswer { result, observed: Vec::new() }
}

fn read<T: serde::de::DeserializeOwned>(body: &[u8]) -> Result<T, MediaFailure> {
    serde_json::from_slice(body).map_err(|error| unreadable(format!("its interaction could not be read: {error}")))
}

#[derive(Debug, Deserialize)]
pub(crate) struct Interaction {
    #[serde(default)]
    pub(crate) id: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    steps: Vec<Step>,
    #[serde(default)]
    pub(crate) usage: Option<Usage>,
    #[serde(default)]
    errors: Vec<Said>,
    #[serde(default)]
    error: Option<Said>,
}

/// Where an interaction stands.
pub(crate) enum Standing {
    /// Waiting to start; `running` once it started.
    Pending { running: bool },
    Completed,
    /// It ended without what it was asked for, with what Google said.
    Failed(VendorTrace),
}

impl Interaction {
    pub(crate) fn standing(&self) -> Standing {
        match self.status.as_deref() {
            Some("completed") => Standing::Completed,
            Some("failed" | "cancelled" | "incomplete") => Standing::Failed(self.failure()),
            Some("queued") | None => Standing::Pending { running: false },
            Some(_) => Standing::Pending { running: true },
        }
    }

    /// What Google said of each of its errors.
    fn failure(&self) -> VendorTrace {
        let errors: Vec<&Said> = self.errors.iter().chain(&self.error).collect();
        let words: Vec<String> = errors.iter().filter_map(|said| said.message.clone()).collect();
        let code = errors.iter().find_map(|said| said.code.clone());
        VendorTrace { words: None, code, id: None }.with_words(Some(words.join("; ")))
    }

    /// The content of its `model_output` steps, in order.
    pub(crate) fn outputs(&self) -> impl DoubleEndedIterator<Item = &Content> {
        self.steps.iter().filter(|step| step.kind == "model_output").flat_map(|step| &step.content)
    }

    /// What the model said in text instead of what it was asked for, if
    /// anything.
    pub(crate) fn said(&self) -> Option<String> {
        let said: Vec<&str> = self.outputs().filter_map(|content| content.text.as_deref()).collect();
        (!said.is_empty()).then(|| said.join(" "))
    }
}

#[derive(Debug, Deserialize)]
struct Step {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    content: Vec<Content>,
}

/// A block of an output: text, or a file within it or at an address.
#[derive(Debug, Deserialize)]
pub(crate) struct Content {
    #[serde(rename = "type")]
    pub(crate) kind: String,
    #[serde(default)]
    pub(crate) data: Option<B64Bytes>,
    #[serde(default)]
    pub(crate) uri: Option<String>,
    #[serde(default)]
    pub(crate) mime_type: Option<String>,
    #[serde(default)]
    pub(crate) text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Said {
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    code: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct Usage {
    #[serde(default)]
    pub(crate) total_input_tokens: u64,
    #[serde(default)]
    pub(crate) total_output_tokens: u64,
    #[serde(default)]
    pub(crate) total_thought_tokens: u64,
    #[serde(default)]
    pub(crate) output_tokens_by_modality: Vec<ModalityTokens>,
}

impl Usage {
    /// The counts, thinking among the output, as the ledger keeps them.
    pub(crate) fn counts(&self) -> TokenUsage {
        TokenUsage {
            input_tokens: self.total_input_tokens,
            output_tokens: self.total_output_tokens + self.total_thought_tokens,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
        }
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct ModalityTokens {
    pub(crate) modality: String,
    pub(crate) tokens: u64,
}

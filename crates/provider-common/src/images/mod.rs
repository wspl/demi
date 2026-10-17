//! The image role of a provider (`images.md` § Vendors): a provider that
//! makes images lists its image models and makes one image for its account
//! or key; the backend chooses the entry and the account. The request a
//! vendor takes is the provider's own; what every vendor's call has in
//! common is in [`crate::media`], and the answer of the OpenAI-shaped image
//! APIs is here.

pub mod openai;
pub mod xai;

use std::fmt;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use bytes::Bytes;
use demi_shared_types::{B64Bytes, TokenUsage};
use futures_util::future::BoxFuture;
use serde::Deserialize;

use crate::media::{MediaAnswer, MediaFailure, MediaMaker, unreadable};
use crate::media_catalog::Catalog;

/// A provider's image role.
pub trait ImageMaker: MediaMaker {
    /// Makes one image for the provider's account or key, sending with
    /// `http`, the client of the shard that asks; `catalog` says what the
    /// model takes.
    fn make<'a>(
        &'a self,
        http: &'a crate::ProviderHttp,
        catalog: &'a Catalog,
        request: ImageRequest,
    ) -> BoxFuture<'a, MediaAnswer<MadeImage>>;
}

/// The shape asked for, such as 16:9, which the plugin read and checks
/// the result against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Aspect {
    pub width: u32,
    pub height: u32,
}

impl Aspect {
    /// A size of this shape whose longer side is `long` pixels, each side a
    /// multiple of 16.
    pub fn size(&self, long: u32) -> (u32, u32) {
        let side = |part: u32, whole: u32| {
            let exact = f64::from(long) * f64::from(part) / f64::from(whole);
            ((exact / 16.0).round() as u32).max(1) * 16
        };
        if self.width >= self.height {
            (long, side(self.height, self.width))
        } else {
            (side(self.width, self.height), long)
        }
    }
}

impl fmt::Display for Aspect {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}:{}", self.width, self.height)
    }
}

/// One image to make.
#[derive(Debug, Clone)]
pub struct ImageRequest {
    pub model: String,
    pub prompt: String,
    pub aspect: Option<Aspect>,
    pub transparent: bool,
    /// The images an edit changes, combines or follows; none for a
    /// generation.
    pub images: Vec<InputImage>,
}

/// An image an edit sends.
#[derive(Debug, Clone)]
pub struct InputImage {
    /// `image/png`, `image/jpeg` or `image/webp`.
    pub media_type: &'static str,
    pub bytes: Bytes,
}

impl InputImage {
    /// The image as a `data:` URL, as the OpenAI-shaped APIs take an
    /// edit's images.
    pub fn data_url(&self) -> String {
        format!("data:{};base64,{}", self.media_type, self.base64())
    }

    /// The image's bytes in base64, as Gemini's inline data takes them.
    pub fn base64(&self) -> String {
        STANDARD.encode(&self.bytes)
    }
}

/// The image a vendor made, with what the usage ledger keeps of the call.
#[derive(Debug, Clone)]
pub struct MadeImage {
    pub bytes: Bytes,
    /// The counts the vendor reported; none when it reported none.
    pub usage: Option<TokenUsage>,
    /// The vendor's id of the image, when it gives one.
    pub vendor_id: Option<String>,
}

/// The answer of the OpenAI-shaped image APIs, OpenAI's, xAI's and the
/// subscriptions': the image as base64, and the counts the vendor reports.
#[derive(Deserialize)]
struct Answer {
    data: Vec<Datum>,
    #[serde(default)]
    usage: Option<AnswerUsage>,
}

#[derive(Deserialize)]
struct Datum {
    b64_json: B64Bytes,
    #[serde(default)]
    generation_id: Option<String>,
}

#[derive(Deserialize)]
struct AnswerUsage {
    #[serde(default)]
    input_tokens: Option<u64>,
    #[serde(default)]
    output_tokens: Option<u64>,
}

/// Reads the answer of an OpenAI-shaped image API: `data[0].b64_json`, and
/// `usage`'s counts when it gives both.
pub fn read_data_answer(text: &[u8]) -> Result<MadeImage, MediaFailure> {
    let answer: Answer = serde_json::from_slice(text)
        .map_err(|error| unreadable(format!("its answer could not be read: {error}")))?;
    let Some(datum) = answer.data.into_iter().next() else {
        return Err(unreadable("its answer held no image".to_owned()));
    };
    let usage = answer.usage.and_then(|usage| {
        Some(TokenUsage {
            input_tokens: usage.input_tokens?,
            output_tokens: usage.output_tokens?,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
        })
    });
    Ok(MadeImage {
        bytes: datum.b64_json.into_bytes(),
        usage,
        vendor_id: datum.generation_id,
    })
}

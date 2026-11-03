//! The models OpenAI lists for a key: `GET …/models` with the key as a
//! bearer token ([List models](https://platform.openai.com/docs/api-reference/models/list)).

use demi_provider_common::ProviderHttp;
use reqwest::Url;
use reqwest::header::{AUTHORIZATION, HeaderValue};
use serde::Deserialize;

#[derive(Deserialize)]
struct Listing {
    data: Vec<Listed>,
}

#[derive(Deserialize)]
struct Listed {
    id: String,
}

/// The ids of every model `url` lists for the key in `authorization`.
pub(crate) async fn list(http: &ProviderHttp, url: &Url, authorization: Option<&HeaderValue>) -> Result<Vec<String>, String> {
    let failed = |cause: String| format!("OpenAI's model list could not be read: {cause}");
    let mut send = http.get(url.clone());
    if let Some(authorization) = authorization {
        send = send.header(AUTHORIZATION, authorization.clone());
    }
    let response = send.send().await.map_err(|error| failed(error.without_url().to_string()))?;
    let status = response.status();
    if !status.is_success() {
        return Err(failed(format!("HTTP {}", status.as_u16())));
    }
    let body = response.bytes().await.map_err(|error| failed(error.without_url().to_string()))?;
    let listing: Listing = serde_json::from_slice(&body).map_err(|error| failed(error.to_string()))?;
    Ok(listing.data.into_iter().map(|model| model.id).collect())
}

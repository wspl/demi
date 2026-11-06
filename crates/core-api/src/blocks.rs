//! A plugin's reply block as a page of its own: the HTML document its plugin
//! answers for the block's id while it is on (`ReplyBlockDocument` in
//! `crates/plugin-interface/src/request.rs`), which the block's component
//! frames.
//!
//! - `GET /api/plugins/:plugin/blocks/:tag/:id`, for the caller.
//! - `POST /api/reply-blocks`, the facts of blocks a page holds replies naming
//!   but no facts of, which a transcript's pages and frames bring otherwise.
//!
//! A share's block is served as its owner's is, by the server
//! (`crates/backend-http/src/shared_blocks.rs`).
//!
//! A document is served as a page and never framed by its parent's own
//! markup, as a `srcdoc` would be: a framed document of its own keeps its own
//! policy, so a shared page's, which allows no inline script, never reaches
//! it. Every answer is sandboxed into an opaque origin (`html_view::headers`),
//! so the document's scripts reach nothing of the product's.

use std::collections::BTreeSet;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use demi_core::conversation::reply_block_facts;
use demi_web_api_protocol::plugins::{ReplyBlocksAnswer, ReplyBlocksRead};

use crate::api::Api;
use crate::body::JsonBody;
use crate::error::ApiError;
use crate::caller::Principal;
use crate::html_view;

/// `GET /plugins/:plugin/blocks/:tag/:id`.
pub(crate) async fn document(
    State(state): State<Api>,
    Principal(user): Principal,
    Path((plugin, tag, id)): Path<(String, String, String)>,
) -> Response {
    let answer = state
        .shards
        .of(&user)
        .call(move |core, cancel| async move { core.plugins().reply_block_document(&plugin, &tag, id, cancel).await })
        .await;
    let answer = match answer {
        Ok(Ok(answer)) => answer,
        Ok(Err(error)) => return unavailable(crate::plugins::refused(error).into_response().status()).await,
        Err(error) => return unavailable(ApiError::from(error).into_response().status()).await,
    };
    served(answer).await
}

/// `document` as a sandboxed page; a block the user does not have, or one
/// with no document, is not found.
pub async fn served(document: Option<String>) -> Response {
    let Some(document) = document else {
        return unavailable(StatusCode::NOT_FOUND).await;
    };
    let mut response = document.into_response();
    let headers = response.headers_mut();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"));
    // A block's document does not change, but it goes with its block: a
    // reader asks again rather than keep one its owner removed.
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("private, no-cache"));
    html_view::headers(response).await
}

/// `POST /reply-blocks { blocks }`: the facts of the user's reply blocks
/// `blocks` names, as their plugins answer them.
pub(crate) async fn facts(
    State(state): State<Api>,
    Principal(user): Principal,
    JsonBody(read): JsonBody<ReplyBlocksRead>,
) -> Result<Json<ReplyBlocksAnswer>, ApiError> {
    let named: BTreeSet<(String, String)> = read.blocks.into_iter().map(|block| (block.tag, block.id)).collect();
    let reply_blocks = state
        .shards
        .of(&user)
        .call(move |core, _| async move { reply_block_facts(core.plugins(), named).await })
        .await?;
    Ok(Json(ReplyBlocksAnswer { reply_blocks }))
}

/// What a block's frame loads when its document cannot be served, with the
/// refusal's status: a page that tells the frame's parent so
/// (`{ replyBlock: "unavailable" }`), whose component says it in its own
/// words, in place of an error's JSON drawn as the block.
pub async fn unavailable(status: StatusCode) -> Response {
    const PAGE: &str = "<!doctype html><meta charset=\"utf-8\"><script>parent.postMessage({ replyBlock: 'unavailable' }, '*')</script>";
    let mut response = (status, PAGE).into_response();
    let headers = response.headers_mut();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"));
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    html_view::headers(response).await
}

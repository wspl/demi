//! A plugin's reply block as a page of its own: the HTML document its plugin
//! answers for the block's id while it is on (`ReplyBlockDocument` in
//! `crates/plugin-interface/src/request.rs`), which the block's component
//! frames.
//!
//! - `GET /api/plugins/:plugin/blocks/:tag/:id`, for the signed-in user.
//! - `POST /api/reply-blocks`, the facts of blocks a page holds replies naming
//!   but no facts of, which a transcript's pages and frames bring otherwise.
//! - `GET /api/shares/:token/plugins/:plugin/blocks/:tag/:id`, for a block a
//!   text of the share names, as the share's owner's: it sends the frame on
//!   to a link of that block alone, `GET /api/blocks/:link`, which serves
//!   it. A document's scripts read the address they load from, and the
//!   share's token there would hand them the whole share; a link reaches
//!   only its block, and only for a minute ([`LINK_LIFETIME`]).
//!
//! A document is served as a page and never framed by its parent's own
//! markup, as a `srcdoc` would be: a framed document of its own keeps its own
//! policy, so a shared page's, which allows no inline script, never reaches
//! it. Every answer is sandboxed into an opaque origin (`html_view::headers`),
//! so the document's scripts reach nothing of the product's.

use std::collections::{BTreeSet, HashMap};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::{Path, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, LOCATION};
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use demi_backend_user_shard::conversation::reply_block_facts;
use demi_web_api_protocol::plugins::{ReplyBlocksAnswer, ReplyBlocksRead};

use super::AppState;
use super::body::JsonBody;
use super::error::ApiError;
use super::gate::AuthUser;
use super::html_view;
use super::shares;

/// `GET /plugins/:plugin/blocks/:tag/:id`.
pub(super) async fn document(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path((plugin, tag, id)): Path<(String, String, String)>,
) -> Response {
    let answer = state
        .shards
        .of(&user.id)
        .call(move |shard, cancel| async move { shard.plugins().reply_block_document(&plugin, &tag, id, cancel).await })
        .await;
    let answer = match answer {
        Ok(Ok(answer)) => answer,
        Ok(Err(error)) => return unavailable(super::plugins::refused(error).into_response().status()).await,
        Err(error) => return unavailable(ApiError::from(error).into_response().status()).await,
    };
    served(answer).await
}

/// How long a shared block's link stays good. The frame that asked for it
/// loads it at once, and loads it again only to redraw in a new appearance,
/// which asks for a new one; the limit is what keeps the links few.
const LINK_LIFETIME: Duration = Duration::from_secs(60);

/// The links of shared reply blocks, in memory: a start ends them all, and
/// a frame asks again.
#[derive(Default)]
pub struct BlockLinks {
    links: Mutex<HashMap<String, BlockLink>>,
}

/// The block a link reaches.
struct BlockLink {
    share: String,
    plugin: String,
    tag: String,
    id: String,
    made: Instant,
}

impl BlockLinks {
    /// A new link of share `share`'s block, ending those past their time.
    fn make(&self, link: BlockLink) -> String {
        let name: String = rand::random::<[u8; 32]>().iter().map(|byte| format!("{byte:02x}")).collect();
        let mut links = self.links.lock().expect("the block links are intact");
        links.retain(|_, held| held.made.elapsed() < LINK_LIFETIME);
        links.insert(name.clone(), link);
        name
    }

    /// The block link `name` reaches while it is good.
    fn reached(&self, name: &str) -> Option<(String, String, String, String)> {
        let links = self.links.lock().expect("the block links are intact");
        let link = links.get(name).filter(|link| link.made.elapsed() < LINK_LIFETIME)?;
        Some((link.share.clone(), link.plugin.clone(), link.tag.clone(), link.id.clone()))
    }
}

/// `GET /shares/:token/plugins/:plugin/blocks/:tag/:id`: on to a link of the
/// block alone, for a share that exists; the link's route answers whether
/// the share shows the block.
pub(super) async fn shared_document(
    State(state): State<AppState>,
    Path((token, plugin, tag, id)): Path<(String, String, String, String)>,
) -> Response {
    if let Err(error) = shares::found(&state.services, token.clone()).await {
        return unavailable(error.into_response().status()).await;
    }
    let link = state.blocks.make(BlockLink { share: token, plugin, tag, id, made: Instant::now() });
    let mut response = StatusCode::SEE_OTHER.into_response();
    let location = HeaderValue::from_str(&format!("/api/blocks/{link}")).expect("a link's name is hexadecimal");
    response.headers_mut().insert(LOCATION, location);
    response
}

/// `GET /blocks/:link`: the document of the shared block `link` reaches.
pub(super) async fn linked_document(State(state): State<AppState>, Path(link): Path<String>) -> Response {
    let Some((token, plugin, tag, id)) = state.blocks.reached(&link) else {
        return unavailable(StatusCode::NOT_FOUND).await;
    };
    match shares::block_document(&state, token, plugin, tag, id).await {
        Ok(document) => served(document).await,
        Err(error) => unavailable(error.into_response().status()).await,
    }
}

/// `document` as a sandboxed page; a block the user does not have, or one
/// with no document, is not found.
async fn served(document: Option<String>) -> Response {
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
pub(super) async fn facts(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    JsonBody(read): JsonBody<ReplyBlocksRead>,
) -> Result<Json<ReplyBlocksAnswer>, ApiError> {
    let named: BTreeSet<(String, String)> = read.blocks.into_iter().map(|block| (block.tag, block.id)).collect();
    let reply_blocks = state
        .shards
        .of(&user.id)
        .call(move |shard, _| async move { reply_block_facts(shard.plugins(), named).await })
        .await?;
    Ok(Json(ReplyBlocksAnswer { reply_blocks }))
}

/// What a block's frame loads when its document cannot be served, with the
/// refusal's status: a page that tells the frame's parent so
/// (`{ replyBlock: "unavailable" }`), whose component says it in its own
/// words, in place of an error's JSON drawn as the block.
async fn unavailable(status: StatusCode) -> Response {
    const PAGE: &str = "<!doctype html><meta charset=\"utf-8\"><script>parent.postMessage({ replyBlock: 'unavailable' }, '*')</script>";
    let mut response = (status, PAGE).into_response();
    let headers = response.headers_mut();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"));
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    html_view::headers(response).await
}

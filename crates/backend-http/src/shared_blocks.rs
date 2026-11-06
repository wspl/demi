//! A reply block of a share as a page of its own.
//!
//! `GET /api/shares/:token/plugins/:plugin/blocks/:tag/:id`, for the block a
//! reply block's share is, or a block a text of the share names, as the
//! share's owner's: it sends the frame on to a link of that block alone,
//! `GET /api/blocks/:link`, which serves it. A document's scripts read the
//! address they load from, and the share's token there would hand them the
//! whole share; a link reaches only its block, and only for a minute
//! ([`LINK_LIFETIME`]). The document is served as the owner's own page
//! serves it (`crates/core-api/src/blocks.rs`).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::extract::{Path, State};
use axum::http::header::LOCATION;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use demi_core_api::blocks::{served, unavailable};

use super::AppState;
use super::shares;

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


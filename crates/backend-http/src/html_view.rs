//! The HTML files the File view shows as pages, from a conversation's files.
//!
//! An HTML file loads from `/api/html/<token>/<place>`, where `<place>` is its
//! path's names from the root of the conversation's Host, so its relative
//! URLs resolve as a web server's would and reach the files around it, as the
//! File view reads them. Every answer
//! keeps what loads in an opaque origin (`sandbox`): its scripts hold no
//! session of the product and read nothing of the page around them. Its
//! requests carry no cookie, which is why a token in the path opens the route,
//! not the session cookie.
//!
//! A token stands for one session's conversation and lives in memory, until a
//! request finds its session ended or the backend stops; the File view asks
//! for the URL each time it shows a file.

use std::collections::HashMap;
use std::sync::Mutex;

use axum::Json;
use axum::extract::{ConnectInfo, Path, State};
use axum::http::header::{
    ACCESS_CONTROL_ALLOW_ORIGIN, CONTENT_SECURITY_POLICY, CONTENT_TYPE, IF_NONE_MATCH, LOCATION,
    RANGE, REFERRER_POLICY, X_CONTENT_TYPE_OPTIONS,
};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use demi_backend_database::accounts::TokenHash;
use demi_backend_host_access::transfer::{Download, DownloadPath, DownloadRequest};
use demi_shared_types::{HTML_SANDBOX, html_view_media_type};
use demi_web_api_protocol::files::{HtmlRequest, HtmlUrl};
use demi_web_api_protocol::ids::ConversationId;
use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};

use super::AppState;
use super::body::JsonBody;
use super::error::ApiError;
use super::files::{conversation_id, download_answer, header};
use super::gate::{AuthUser, CurrentSession};
use super::listener::Peer;

/// What a name in a URL's path keeps encoded: what would end it or start a
/// query or a fragment, a `:`, which would make a redirect's relative URL,
/// `a:b/`, read as a scheme, and what is not printable.
const NAME: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'/')
    .add(b':')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'\\')
    .add(b'^')
    .add(b'`')
    .add(b'{')
    .add(b'|')
    .add(b'}');

/// The tokens of the HTML views, in memory.
#[derive(Default)]
pub struct HtmlGrants {
    grants: Mutex<Grants>,
}

#[derive(Default)]
struct Grants {
    /// What each token stands for.
    by_token: HashMap<String, Grant>,
    /// Each grant's token, so a session's conversation keeps one.
    tokens: HashMap<(String, ConversationId), String>,
}

#[derive(Clone)]
struct Grant {
    session: TokenHash,
    conversation: ConversationId,
}

impl HtmlGrants {
    /// The token of `session`'s grant of `conversation`, made the first time.
    fn token(&self, session: &TokenHash, conversation: &ConversationId) -> String {
        let mut grants = self.grants.lock().expect("the grants are intact");
        let key = (session.as_str().to_owned(), conversation.clone());
        if let Some(token) = grants.tokens.get(&key) {
            return token.clone();
        }
        let token: String = rand::random::<[u8; 32]>()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        grants.by_token.insert(
            token.clone(),
            Grant {
                session: session.clone(),
                conversation: conversation.clone(),
            },
        );
        grants.tokens.insert(key, token.clone());
        token
    }

    fn grant(&self, token: &str) -> Option<Grant> {
        let grants = self.grants.lock().expect("the grants are intact");
        grants.by_token.get(token).cloned()
    }

    /// Ends every grant of `session`, which ended.
    fn end(&self, session: &TokenHash) {
        let mut grants = self.grants.lock().expect("the grants are intact");
        grants.by_token.retain(|_, grant| grant.session != *session);
        grants.tokens.retain(|(held, _), _| held != session.as_str());
    }
}

/// `POST /conversations/:id/fs/html { path }`: where the HTML file at `path`
/// loads.
pub(super) async fn grant(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    session: CurrentSession,
    Path(id): Path<String>,
    JsonBody(HtmlRequest { path }): JsonBody<HtmlRequest>,
) -> Result<Json<HtmlUrl>, ApiError> {
    let conversation = conversation_id(&id)?;
    let asked = conversation.clone();
    let place = state
        .shards
        .of(&user.id)
        .call(move |shard, cancel| async move {
            shard
                .host_shard()
                .html_place(&asked, path.as_str(), &cancel)
                .await
        })
        .await??;
    let token = state.html.token(&session.token, &conversation);
    let place: Vec<String> = place
        .split('/')
        .map(|name| utf8_percent_encode(name, NAME).to_string())
        .collect();
    Ok(Json(HtmlUrl {
        url: format!("/api/html/{token}/{}", place.join("/")),
    }))
}

/// `GET` and `HEAD /api/html/:token/*place`: the file at `place` on the Host
/// of the conversation the token's session may read, a folder's `index.html`
/// for a folder.
pub(super) async fn serve(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<Peer>,
    method: Method,
    headers: HeaderMap,
    Path((token, place)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    answer(state, peer, method, headers, token, place).await
}

/// `GET` and `HEAD /api/html/:token/`: the `index.html` at the root of a
/// Posix Host.
pub(super) async fn serve_root(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<Peer>,
    method: Method,
    headers: HeaderMap,
    Path(token): Path<String>,
) -> Result<Response, ApiError> {
    answer(state, peer, method, headers, token, String::new()).await
}

async fn answer(
    state: AppState,
    peer: Peer,
    method: Method,
    headers: HeaderMap,
    token: String,
    mut place: String,
) -> Result<Response, ApiError> {
    let grant = state.html.grant(&token).ok_or_else(not_found)?;
    let Some(session) = state.services.sessions.check(&grant.session).await? else {
        state.html.end(&grant.session);
        return Err(not_found());
    };
    // A folder is its `index.html`, once: a folder named so is no file.
    for _ in 0..2 {
        let conversation = grant.conversation.clone();
        let request = DownloadRequest {
            path: DownloadPath::Place(place.clone()),
            version: None,
            head: method == Method::HEAD,
            range: header(&headers, RANGE),
            if_none_match: header(&headers, IF_NONE_MATCH),
        };
        let download = state
            .shards
            .of(&session.user.id)
            .call(move |shard, cancel| async move {
                shard
                    .host_shard()
                    .open_download(&conversation, request, &cancel)
                    .await
            })
            .await??;
        if !matches!(download, Download::NotAFile) {
            return download_answer(download, &peer, false, content_type(&place));
        }
        if !(place.is_empty() || place.ends_with('/')) {
            // Its relative URLs start inside it once its URL ends with `/`.
            let name = place.rsplit('/').next().unwrap_or_default();
            let location = format!("{}/", utf8_percent_encode(name, NAME));
            let location =
                HeaderValue::from_str(&location).expect("an encoded name is a header value");
            return Ok((StatusCode::PERMANENT_REDIRECT, [(LOCATION, location)]).into_response());
        }
        place.push_str("index.html");
    }
    Err(not_found())
}

/// The type of the file at `place`, among the web's.
fn content_type(place: &str) -> HeaderMap {
    let media_type = html_view_media_type(place).unwrap_or("application/octet-stream");
    let value = HeaderValue::from_str(media_type)
        .expect("the file-type table's media types are header values");
    HeaderMap::from_iter([(CONTENT_TYPE, value)])
}

/// What every answer of the route carries, a refusal or a redirect too: the
/// sandbox; any origin, which an opaque one is, so a page reads a 404 as a
/// browser tab would; no referrer, which would carry the token; and
/// `nosniff`.
pub(super) async fn headers(mut response: Response) -> Response {
    let policy = format!("sandbox {HTML_SANDBOX}");
    let headers = response.headers_mut();
    headers.insert(
        CONTENT_SECURITY_POLICY,
        HeaderValue::from_str(&policy).expect("the sandbox is a header value"),
    );
    headers.insert(ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
    headers.insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    response
}

fn not_found() -> ApiError {
    ApiError::new(
        StatusCode::NOT_FOUND,
        demi_web_api_protocol::error::ErrorCode::NotFound,
        "No such file",
    )
}

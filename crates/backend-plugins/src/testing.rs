//! What the tests of the plugins' outbound requests and OAuth sign-ins run
//! against. No real vendor is reached.
//!
//! The module provides:
//!
//! - A certificate authority of the test's own.
//! - An OAuth 2.1 authorization server over HTTPS. It serves its metadata (RFC
//!   8414), registers clients (RFC 7591), redirects an authorization with its
//!   code, issues tokens for a code whose PKCE verifier matches (RFC 7636) and
//!   for a refresh token spent once, and revokes tokens (RFC 7009).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::extract::{Form, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use demi_provider_common::testing::TlsListener;
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::sync::{Notify, watch};
use tokio_util::task::AbortOnDropHandle;
use url::Url;

/// A certificate authority of the test's, and a certificate it issued for
/// `127.0.0.1`.
pub struct Authority {
    ca: String,
    certificate: String,
    key: String,
}

impl Authority {
    pub fn new() -> Self {
        use rcgen::{BasicConstraints, CertificateParams, IsCa, Issuer, KeyPair};

        let ca_key = KeyPair::generate().expect("a key pair generates");
        let mut ca_params = CertificateParams::new(Vec::<String>::new()).expect("CA parameters");
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        let ca = ca_params.self_signed(&ca_key).expect("the CA signs itself");
        let issuer = Issuer::new(ca_params, ca_key);
        let key = KeyPair::generate().expect("a key pair generates");
        let params = CertificateParams::new(vec!["127.0.0.1".to_owned()])
            .expect("the server's parameters");
        let certificate = params
            .signed_by(&key, &issuer)
            .expect("the CA signs the server's certificate");
        Self {
            ca: ca.pem(),
            certificate: certificate.pem(),
            key: key.serialize_pem(),
        }
    }

    /// The authority as a client trusts it.
    pub fn root(&self) -> reqwest::Certificate {
        reqwest::Certificate::from_pem(self.ca.as_bytes()).expect("the CA is PEM")
    }

    /// A client that trusts this authority, sends only `https` and follows
    /// no redirect, as the plugins' client does but for the redirects.
    pub fn client(&self) -> reqwest::Client {
        reqwest::Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .add_root_certificate(self.root())
            .build()
            .expect("the test's client builds")
    }

    /// Serves `router` over HTTPS with this authority's certificate, until
    /// the returned handle is dropped; answers its base URL with it.
    pub async fn serve(&self, router: Router) -> (Url, AbortOnDropHandle<()>) {
        let listener = TlsListener::bind(self.certificate.as_bytes(), self.key.as_bytes()).await;
        let address = axum::serve::Listener::local_addr(&listener).expect("a bound listener");
        let server = tokio::spawn(async move {
            // The server ends when the test drops it.
            let _ = axum::serve(listener, router).await;
        });
        let url = Url::parse(&format!("https://{address}")).expect("an address is a URL");
        (url, AbortOnDropHandle::new(server))
    }
}

impl Default for Authority {
    fn default() -> Self {
        Self::new()
    }
}

/// The fake authorization server.
pub struct AuthorizationServer {
    issuer: Url,
    state: Arc<Mutex<Server>>,
    /// Woken at each refresh that arrives.
    refreshed: Arc<Notify>,
    /// Holds each refresh's answer while it reads true.
    held: watch::Sender<bool>,
    client: reqwest::Client,
    _server: AbortOnDropHandle<()>,
}

#[derive(Default)]
struct Server {
    issuer: String,
    /// Each registered client's redirect URIs, by its id.
    clients: HashMap<String, Vec<String>>,
    codes: HashMap<String, Grant>,
    /// Each refresh token's client, and whether it was spent.
    refresh_tokens: HashMap<String, (String, bool)>,
    issued: u32,
    registrations: u32,
    refreshes: u32,
    revoked: Vec<String>,
    /// The lifetime of each access token issued, in seconds.
    expires_in: u64,
    refuse_refreshes: bool,
    /// What every refresh is answered while set: a status and a body,
    /// without spending the refresh token.
    failing_refreshes: Option<(StatusCode, String)>,
    /// Whether a refresh token stays good after it was used, as at a
    /// vendor that does not rotate them.
    reusable_refresh_tokens: bool,
    revocation: bool,
}

struct Grant {
    client_id: String,
    redirect_uri: String,
    challenge: String,
    resource: Option<String>,
}

type Shared = (Arc<Mutex<Server>>, watch::Receiver<bool>, Arc<Notify>);

impl AuthorizationServer {
    /// A server with the certificate `authority` issued, which issues
    /// tokens that live an hour and offers revocation.
    pub async fn start(authority: &Authority) -> Self {
        let state = Arc::new(Mutex::new(Server {
            expires_in: 3600,
            revocation: true,
            ..Server::default()
        }));
        let (held, holds) = watch::channel(false);
        let refreshed = Arc::new(Notify::new());
        let router = Router::new()
            .route("/.well-known/oauth-authorization-server", get(metadata))
            .route("/register", post(register))
            .route("/authorize", get(authorize))
            .route("/token", post(token))
            .route("/revoke", post(revoke))
            .with_state((state.clone(), holds, refreshed.clone()));
        let (issuer, server) = authority.serve(router).await;
        state.lock().unwrap().issuer = issuer.as_str().trim_end_matches('/').to_owned();
        Self {
            issuer,
            state,
            refreshed,
            held,
            client: authority.client(),
            _server: server,
        }
    }

    pub fn issuer(&self) -> Url {
        self.issuer.clone()
    }

    fn server(&self) -> std::sync::MutexGuard<'_, Server> {
        self.state.lock().unwrap()
    }

    /// Acts as the user's browser at the authorization URL: approves the
    /// sign-in and answers where the server sent the browser back.
    pub async fn approve(&self, authorization_url: &str) -> Url {
        let response = self.client.get(authorization_url).send().await.unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::FOUND);
        let location = response.headers()[reqwest::header::LOCATION]
            .to_str()
            .unwrap()
            .to_owned();
        Url::parse(&location).unwrap()
    }

    /// How many clients registered.
    pub fn registrations(&self) -> u32 {
        self.server().registrations
    }

    /// How many refreshes were asked for, granted or not.
    pub fn refreshes(&self) -> u32 {
        self.server().refreshes
    }

    /// Resolves once `count` refreshes arrived.
    pub async fn until_refreshes(&self, count: u32) {
        loop {
            let refreshed = self.refreshed.notified();
            let mut refreshed = std::pin::pin!(refreshed);
            refreshed.as_mut().enable();
            if self.refreshes() >= count {
                return;
            }
            refreshed.await;
        }
    }

    /// The tokens revoked, in order.
    pub fn revoked(&self) -> Vec<String> {
        self.server().revoked.clone()
    }

    /// Whether `token` is a refresh token the server issued and nobody
    /// spent yet.
    pub fn live_refresh_token(&self, token: &str) -> bool {
        self.server()
            .refresh_tokens
            .get(token)
            .is_some_and(|(_, spent)| !spent)
    }

    /// The access tokens issued from now on live `seconds`.
    pub fn expire_in(&self, seconds: u64) {
        self.server().expires_in = seconds;
    }

    /// Every refresh from now on is refused.
    pub fn refuse_refreshes(&self) {
        self.server().refuse_refreshes = true;
    }

    /// Every refresh from now on is answered `status` with `body`, and
    /// spends nothing, until this is called with `None`.
    pub fn fail_refreshes(&self, answer: Option<(u16, &str)>) {
        self.server().failing_refreshes = answer.map(|(status, body)| {
            (StatusCode::from_u16(status).unwrap(), body.to_owned())
        });
    }

    /// A refresh token stays good after it was used from now on, as at a
    /// vendor that does not rotate them.
    pub fn reuse_refresh_tokens(&self) {
        self.server().reusable_refresh_tokens = true;
    }

    /// The server forgets every client it registered.
    pub fn forget_clients(&self) {
        self.server().clients.clear();
    }

    /// Holds the answers to refreshes while `held` is true.
    pub fn hold_refreshes(&self, held: bool) {
        self.held.send_replace(held);
    }
}

async fn metadata(State((state, _, _)): State<Shared>) -> Response {
    let server = state.lock().unwrap();
    let issuer = &server.issuer;
    let mut metadata = json!({
        "issuer": issuer,
        "authorization_endpoint": format!("{issuer}/authorize"),
        "token_endpoint": format!("{issuer}/token"),
        "registration_endpoint": format!("{issuer}/register"),
        "code_challenge_methods_supported": ["S256"],
        "response_types_supported": ["code"],
    });
    if server.revocation {
        metadata["revocation_endpoint"] = json!(format!("{issuer}/revoke"));
    }
    Json(metadata).into_response()
}

async fn register(
    State((state, _, _)): State<Shared>,
    Json(registration): Json<serde_json::Value>,
) -> Response {
    let mut server = state.lock().unwrap();
    assert_eq!(registration["token_endpoint_auth_method"], "none");
    let uris: Vec<String> = serde_json::from_value(registration["redirect_uris"].clone()).unwrap();
    server.registrations += 1;
    let client_id = format!("client-{}", server.registrations);
    server.clients.insert(client_id.clone(), uris);
    (StatusCode::CREATED, Json(json!({ "client_id": client_id }))).into_response()
}

async fn authorize(
    State((state, _, _)): State<Shared>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let mut server = state.lock().unwrap();
    let client_id = &query["client_id"];
    let redirect_uri = &query["redirect_uri"];
    let known = server
        .clients
        .get(client_id)
        .is_some_and(|uris| uris.contains(redirect_uri));
    if !known || query["code_challenge_method"] != "S256" || query["response_type"] != "code" {
        return (StatusCode::BAD_REQUEST, "unknown client").into_response();
    }
    server.issued += 1;
    let code = format!("code-{}", server.issued);
    server.codes.insert(
        code.clone(),
        Grant {
            client_id: client_id.clone(),
            redirect_uri: redirect_uri.clone(),
            challenge: query["code_challenge"].clone(),
            resource: query.get("resource").cloned(),
        },
    );
    let mut location = Url::parse(redirect_uri).unwrap();
    location
        .query_pairs_mut()
        .append_pair("code", &code)
        .append_pair("state", &query["state"]);
    (StatusCode::FOUND, [(header::LOCATION, location.to_string())]).into_response()
}

async fn token(
    State((state, mut holds, refreshed)): State<Shared>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let client_id = form.get("client_id").cloned().unwrap_or_default();
    if form["grant_type"] == "refresh_token" {
        state.lock().unwrap().refreshes += 1;
        refreshed.notify_waiters();
        // The test holds the answer, the request having arrived.
        let _ = holds.wait_for(|held| !held).await;
    }
    let mut server = state.lock().unwrap();
    if !server.clients.contains_key(&client_id) {
        return refusal(StatusCode::UNAUTHORIZED, "invalid_client");
    }
    match form["grant_type"].as_str() {
        "authorization_code" => {
            let Some(grant) = server.codes.remove(&form["code"]) else {
                return refusal(StatusCode::BAD_REQUEST, "invalid_grant");
            };
            let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(form["code_verifier"].as_bytes()));
            let valid = grant.client_id == client_id
                && grant.redirect_uri == form["redirect_uri"]
                && grant.challenge == challenge
                && grant.resource.as_ref() == form.get("resource");
            if !valid {
                return refusal(StatusCode::BAD_REQUEST, "invalid_grant");
            }
            issue(&mut server, &client_id)
        }
        "refresh_token" => {
            if let Some((status, body)) = server.failing_refreshes.clone() {
                return (status, body).into_response();
            }
            let refuse = server.refuse_refreshes;
            let reusable = server.reusable_refresh_tokens;
            let Some((owner, spent)) = server.refresh_tokens.get_mut(&form["refresh_token"]) else {
                return refusal(StatusCode::BAD_REQUEST, "invalid_grant");
            };
            if *spent || *owner != client_id || refuse {
                return refusal(StatusCode::BAD_REQUEST, "invalid_grant");
            }
            *spent = !reusable;
            issue(&mut server, &client_id)
        }
        _ => refusal(StatusCode::BAD_REQUEST, "unsupported_grant_type"),
    }
}

/// New tokens for `client_id`.
fn issue(server: &mut Server, client_id: &str) -> Response {
    server.issued += 1;
    let access = format!("access-{}", server.issued);
    let refresh = format!("refresh-{}", server.issued);
    server
        .refresh_tokens
        .insert(refresh.clone(), (client_id.to_owned(), false));
    Json(json!({
        "access_token": access,
        "refresh_token": refresh,
        "token_type": "Bearer",
        "expires_in": server.expires_in,
    }))
    .into_response()
}

fn refusal(status: StatusCode, error: &str) -> Response {
    (status, Json(json!({ "error": error }))).into_response()
}

async fn revoke(
    State((state, _, _)): State<Shared>,
    Form(form): Form<HashMap<String, String>>,
) -> StatusCode {
    let mut server = state.lock().unwrap();
    let token = form["token"].clone();
    if let Some((_, spent)) = server.refresh_tokens.get_mut(&token) {
        *spent = true;
    }
    server.revoked.push(token);
    StatusCode::OK
}

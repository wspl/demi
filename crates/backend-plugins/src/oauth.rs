//! The plugins' OAuth sign-ins (`plugins.md` § OAuth sign-in): for a plugin
//! whose manifest names its vendor's authorization server, the host
//! registers Demi as a public client once per instance and issuer (RFC
//! 7591), starts a user's sign-in with PKCE (RFC 7636, S256) and a `state`
//! kept with the user for 10 minutes, exchanges the code the callback
//! brings, keeps the tokens sealed as the plugin's secret under a name no
//! plugin can read, renews them by the provider credentials' protocol
//! (`providers.md` § Token refresh) and revokes them (RFC 7009) at sign
//! out. Every call to the vendor is an outbound request of the plugin
//! (`plugins.md` § Outbound requests); the endpoints are read from the
//! server's metadata (RFC 8414). A plugin sees only the access token.

use std::collections::HashMap;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use demi_backend_database::StorageError;
use demi_backend_database::plugin_secrets::OAuthClient;
use demi_host_interface::PortError;
use demi_plugin_interface::{
    OAuth, PortFailure, PortRefusal, StoredSecret, WebFailure, WebMethod, WebRequest, WebResponse,
};
use demi_provider_common::Secret;
use demi_provider_common::credentials::decode_secret;
use demi_provider_common::oauth::Lifetime;
use demi_shared_types::Timestamp;
use jiff::SignedDuration;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::user::{Shared, storage};
use crate::web;

/// The secret a sign-in's tokens are kept as: a name a plugin cannot give,
/// so no plugin reads its refresh token.
const TOKENS: &str = "oauth:tokens";

/// How long a started sign-in waits for its callback.
const PENDING: SignedDuration = SignedDuration::from_mins(10);

/// An access token that expires within this long is renewed first.
const RENEW_WITHIN: SignedDuration = SignedDuration::from_mins(1);

/// The deadline and body limit of each call to the authorization server.
const VENDOR_DEADLINE: std::time::Duration = std::time::Duration::from_secs(30);
const VENDOR_BODY_BYTES: u64 = 1024 * 1024;

/// The sign-ins the user started and the turns of each plugin's renewals.
pub(crate) struct SignIns {
    /// The sign-ins waiting for their callback, by `state`.
    pending: std::cell::RefCell<HashMap<String, Pending>>,
    /// Each plugin's turn, by its index: its renewals, sign-outs and stores
    /// of new tokens take turns within the shard, so two token calls spend
    /// a refresh token once.
    turns: Vec<tokio::sync::Mutex<()>>,
}

impl SignIns {
    pub(crate) fn new(plugins: usize) -> Self {
        Self {
            pending: std::cell::RefCell::default(),
            turns: (0..plugins).map(|_| tokio::sync::Mutex::new(())).collect(),
        }
    }
}

/// A sign-in waiting for its callback.
struct Pending {
    plugin: usize,
    verifier: String,
    client_id: String,
    redirect_uri: String,
    started: Timestamp,
}

/// The tokens of a user's sign-in, as they are sealed.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Tokens {
    /// The client the tokens were issued to.
    client_id: String,
    access_token: Secret,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    refresh_token: Option<Secret>,
    /// When the access token expires, in milliseconds since the Unix epoch;
    /// none when the vendor did not say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    expires_at: Option<i64>,
}

/// The authorization server's metadata (RFC 8414 § 2), what the flow uses
/// of it.
#[derive(Deserialize)]
struct Metadata {
    issuer: Url,
    authorization_endpoint: Url,
    token_endpoint: Url,
    #[serde(default)]
    registration_endpoint: Option<Url>,
    #[serde(default)]
    revocation_endpoint: Option<Url>,
    #[serde(default)]
    code_challenge_methods_supported: Option<Vec<String>>,
}

/// A registration's answer (RFC 7591 § 3.2.1).
#[derive(Deserialize)]
struct Registered {
    client_id: String,
}

/// A token endpoint's answer (RFC 6749 § 5.1).
#[derive(Deserialize)]
struct Issued {
    access_token: Secret,
    #[serde(default)]
    refresh_token: Option<Secret>,
    #[serde(default)]
    expires_in: Lifetime,
}

/// A token endpoint's refusal (RFC 6749 § 5.2).
#[derive(Deserialize)]
struct Refusal {
    error: String,
}

/// What the browser brings back to the callback (RFC 6749 § 4.1.2). No
/// `Debug`: the code is a credential until it is exchanged.
#[derive(Clone, Default, Deserialize)]
pub struct Callback {
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

/// Why a callback did not sign the user in.
#[derive(Debug, thiserror::Error)]
pub enum CallbackError {
    #[error("No plugin \"{0}\" signs in")]
    UnknownPlugin(String),
    /// The `state` is not one this user started, or it is older than 10
    /// minutes, or the callback brings none.
    #[error("This sign-in was not started here, or it expired")]
    UnknownState,
    /// The vendor sent the browser back with an error, such as
    /// `access_denied`.
    #[error("{vendor} answered {error}")]
    Denied { vendor: String, error: String },
    /// The code could not be exchanged for tokens.
    #[error("{vendor} did not issue the tokens: {failure}")]
    Exchange { vendor: String, failure: PortFailure },
    #[error(transparent)]
    Storage(#[from] StorageError),
}

/// What a token endpoint answered.
enum Exchanged {
    Issued(Issued),
    /// A refusal of the grant or of the client (HTTP 400 or 401); whether
    /// the vendor no longer knows the client.
    Refused { unknown_client: bool },
}

impl Shared {
    fn oauth(&self, plugin: usize) -> Result<&OAuth, PortFailure> {
        self.registry.plugins[plugin]
            .factory
            .manifest()
            .oauth
            .as_ref()
            .ok_or_else(|| PortError::Failed("the plugin declares no OAuth sign-in".into()).into())
    }

    /// Plugin `plugin`'s call to its authorization server.
    async fn vendor(
        &self,
        plugin: usize,
        request: WebRequest,
        cancel: &CancellationToken,
    ) -> Result<WebResponse, PortFailure> {
        let id = self.registry.plugins[plugin].id();
        web::send(&self.product.web_client(), id, request, cancel).await
    }

    /// The authorization server's metadata, at its well-known address with
    /// the issuer's path after it (RFC 8414 § 3.1).
    async fn metadata(
        &self,
        plugin: usize,
        oauth: &OAuth,
        cancel: &CancellationToken,
    ) -> Result<Metadata, PortFailure> {
        let mut url = oauth.issuer.clone();
        let path = oauth.issuer.path().trim_end_matches('/');
        url.set_path(&format!("/.well-known/oauth-authorization-server{path}"));
        let request = WebRequest::new(WebMethod::Get, url, VENDOR_DEADLINE, VENDOR_BODY_BYTES)
            .header("accept", "application/json");
        let response = self.vendor(plugin, request, cancel).await?;
        if !(200..300).contains(&response.status) {
            return Err(vendor(format!(
                "the authorization server's metadata answered HTTP {}",
                response.status
            )));
        }
        let metadata: Metadata = decoded("metadata", &response)?;
        if metadata.issuer != oauth.issuer {
            return Err(vendor(format!(
                "the authorization server names itself {}, not {}",
                metadata.issuer, oauth.issuer
            )));
        }
        let s256 = metadata
            .code_challenge_methods_supported
            .as_ref()
            .is_none_or(|methods| methods.iter().any(|method| method == "S256"));
        if !s256 {
            return Err(vendor("the authorization server does not take S256 PKCE".into()));
        }
        Ok(metadata)
    }

    /// Where the vendor sends the browser back to:
    /// `<public URL>/api/plugins/<plugin>/oauth/callback`.
    fn redirect_uri(&self, plugin: usize) -> Result<String, PortFailure> {
        let public = self.product.public_url().ok_or_else(|| {
            PortFailure::Port(PortError::Failed("the backend has no public URL yet".into()))
        })?;
        Ok(format!(
            "{}/api/plugins/{}/oauth/callback",
            public.as_str().trim_end_matches('/'),
            self.plugin_id(plugin)
        ))
    }

    /// The public client registered for plugin `plugin` at its issuer with
    /// `redirect_uri`, registered now when there is none.
    async fn client(
        &self,
        plugin: usize,
        oauth: &OAuth,
        metadata: &Metadata,
        redirect_uri: &str,
        cancel: &CancellationToken,
    ) -> Result<String, PortFailure> {
        let id = self.plugin_id(plugin);
        let issuer = oauth.issuer.to_string();
        let known = self
            .control
            .plugin_oauth_client(id.clone(), issuer.clone())
            .await
            .map_err(storage)?;
        if let Some(client) = known.filter(|client| client.redirect_uri == redirect_uri) {
            return Ok(client.client_id);
        }
        let Some(endpoint) = metadata.registration_endpoint.clone() else {
            return Err(vendor(
                "the authorization server offers no dynamic client registration".into(),
            ));
        };
        let registration = serde_json::json!({
            "client_name": "Demi",
            "redirect_uris": [redirect_uri],
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
            "token_endpoint_auth_method": "none",
            "scope": oauth.scopes.join(" "),
        });
        let request = WebRequest::new(WebMethod::Post, endpoint, VENDOR_DEADLINE, VENDOR_BODY_BYTES)
            .header("content-type", "application/json")
            .header("accept", "application/json")
            .body(registration.to_string());
        let response = self.vendor(plugin, request, cancel).await?;
        if !(200..300).contains(&response.status) {
            return Err(vendor(format!(
                "the authorization server refused the registration with HTTP {}",
                response.status
            )));
        }
        let registered: Registered = decoded("registration", &response)?;
        let client = OAuthClient {
            redirect_uri: redirect_uri.to_owned(),
            client_id: registered.client_id.clone(),
        };
        self.control
            .set_plugin_oauth_client(id, issuer, client)
            .await
            .map_err(storage)?;
        Ok(registered.client_id)
    }

    /// Forgets the client the vendor no longer knows, so the next sign-in
    /// registers again.
    async fn forget_client(&self, plugin: usize, oauth: &OAuth, client_id: &str) {
        let forgot = self
            .control
            .forget_plugin_oauth_client(
                self.plugin_id(plugin),
                oauth.issuer.to_string(),
                client_id.to_owned(),
            )
            .await;
        if let Err(error) = forgot {
            // The next sign-in uses the stale client and fails again, which
            // tries this again.
            tracing::warn!(plugin = self.plugin_id(plugin), %error, "a forgotten OAuth client stayed");
        }
    }

    /// Starts the user's sign-in to plugin `plugin`'s vendor and answers the
    /// authorization URL.
    pub(crate) async fn oauth_start(
        &self,
        plugin: usize,
        cancel: &CancellationToken,
    ) -> Result<String, PortFailure> {
        let oauth = self.oauth(plugin)?;
        let metadata = self.metadata(plugin, oauth, cancel).await?;
        let redirect_uri = self.redirect_uri(plugin)?;
        let client_id = self
            .client(plugin, oauth, &metadata, &redirect_uri, cancel)
            .await?;
        let verifier = random_text();
        let state = random_text();
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let mut url = metadata.authorization_endpoint;
        {
            let mut query = url.query_pairs_mut();
            query
                .append_pair("response_type", "code")
                .append_pair("client_id", &client_id)
                .append_pair("redirect_uri", &redirect_uri)
                .append_pair("state", &state)
                .append_pair("code_challenge", &challenge)
                .append_pair("code_challenge_method", "S256")
                .append_pair("resource", &oauth.resource);
            if !oauth.scopes.is_empty() {
                query.append_pair("scope", &oauth.scopes.join(" "));
            }
        }
        let now = self.product.now();
        let mut pending = self.sign_ins.pending.borrow_mut();
        pending.retain(|_, waiting| elapsed(now, waiting.started) < PENDING);
        pending.insert(
            state,
            Pending {
                plugin,
                verifier,
                client_id,
                redirect_uri,
                started: now,
            },
        );
        Ok(url.to_string())
    }

    /// The vendor sent the browser back to plugin `plugin`'s callback:
    /// exchanges the code, keeps the tokens and marks the plugin's user
    /// state changed; answers the vendor's name.
    pub(crate) async fn oauth_callback(
        &self,
        plugin: usize,
        callback: Callback,
        cancel: &CancellationToken,
    ) -> Result<String, CallbackError> {
        let id = self.plugin_id(plugin);
        let oauth = self
            .oauth(plugin)
            .map_err(|_| CallbackError::UnknownPlugin(id.clone()))?;
        let name = oauth.vendor.clone();
        // A state is used once, whatever the callback brings with it.
        let pending = callback
            .state
            .and_then(|state| self.sign_ins.pending.borrow_mut().remove(&state))
            .filter(|pending| {
                pending.plugin == plugin
                    && elapsed(self.product.now(), pending.started) < PENDING
            })
            .ok_or(CallbackError::UnknownState)?;
        if let Some(error) = callback.error {
            return Err(CallbackError::Denied { vendor: name, error });
        }
        let code = callback.code.ok_or(CallbackError::UnknownState)?;
        let failed = |failure| CallbackError::Exchange {
            vendor: name.clone(),
            failure,
        };
        let metadata = self
            .metadata(plugin, oauth, cancel)
            .await
            .map_err(failed)?;
        let form = [
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("redirect_uri", pending.redirect_uri.as_str()),
            ("client_id", pending.client_id.as_str()),
            ("code_verifier", pending.verifier.as_str()),
            ("resource", oauth.resource.as_str()),
        ];
        let issued = match self
            .token_endpoint(plugin, &metadata, &form, cancel)
            .await
            .map_err(failed)?
        {
            Exchanged::Issued(issued) => issued,
            Exchanged::Refused { unknown_client } => {
                if unknown_client {
                    self.forget_client(plugin, oauth, &pending.client_id).await;
                }
                return Err(failed(vendor("the vendor refused the code".into())));
            }
        };
        let tokens = self.tokens(pending.client_id, issued, None);
        let _turn = self.sign_ins.turns[plugin].lock().await;
        // A new sign-in replaces whatever the user had.
        loop {
            let revision = self
                .read_secret(plugin, TOKENS)
                .await
                .map_err(failed)?
                .map(|stored| stored.revision);
            match self.write_secret(plugin, TOKENS, &sealed(&tokens), revision).await {
                Ok(_) => break,
                Err(PortFailure::Refused(PortRefusal::Conflict)) => continue,
                Err(failure) => return Err(failed(failure)),
            }
        }
        self.changed(plugin, None);
        Ok(name)
    }

    /// A live access token of the user's sign-in to plugin `plugin`'s
    /// vendor, renewed first when it expires within a minute.
    pub(crate) async fn oauth_token(
        &self,
        plugin: usize,
        cancel: &CancellationToken,
    ) -> Result<String, PortFailure> {
        let oauth = self.oauth(plugin)?;
        let _turn = self.sign_ins.turns[plugin].lock().await;
        let Some((tokens, revision)) = self.stored_tokens(plugin).await? else {
            return Err(PortFailure::Refused(PortRefusal::SignedOut));
        };
        if !self.due(&tokens) {
            return Ok(tokens.access_token.expose().to_owned());
        }
        let Some(refresh) = &tokens.refresh_token else {
            return self.signed_out(plugin, revision).await;
        };
        let metadata = self.metadata(plugin, oauth, cancel).await?;
        let form = [
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh.expose()),
            ("client_id", tokens.client_id.as_str()),
            ("resource", oauth.resource.as_str()),
        ];
        match self.token_endpoint(plugin, &metadata, &form, cancel).await? {
            Exchanged::Issued(issued) => {
                let renewed = self.tokens(tokens.client_id, issued, tokens.refresh_token);
                let access = renewed.access_token.expose().to_owned();
                match self
                    .write_secret(plugin, TOKENS, &sealed(&renewed), Some(revision))
                    .await
                {
                    Ok(_) => Ok(access),
                    // Another worker stored tokens first: they are the live
                    // ones.
                    Err(PortFailure::Refused(PortRefusal::Conflict)) => {
                        match self.stored_tokens(plugin).await? {
                            Some((stored, _)) => Ok(stored.access_token.expose().to_owned()),
                            None => Err(PortFailure::Refused(PortRefusal::SignedOut)),
                        }
                    }
                    Err(failure) => Err(failure),
                }
            }
            Exchanged::Refused { unknown_client } => {
                // Another worker may have spent the refresh token and
                // stored what it got.
                if let Some((stored, moved)) = self.stored_tokens(plugin).await?
                    && moved != revision
                {
                    return Ok(stored.access_token.expose().to_owned());
                }
                if unknown_client {
                    self.forget_client(plugin, oauth, &tokens.client_id).await;
                }
                self.signed_out(plugin, revision).await
            }
        }
    }

    /// Signs the user out of plugin `plugin`'s vendor: forgets the tokens,
    /// then asks the vendor to revoke them when it offers revocation.
    pub(crate) async fn oauth_sign_out(
        &self,
        plugin: usize,
        cancel: &CancellationToken,
    ) -> Result<(), PortFailure> {
        let oauth = self.oauth(plugin)?;
        let _turn = self.sign_ins.turns[plugin].lock().await;
        let tokens = loop {
            let Some((tokens, revision)) = self.stored_tokens(plugin).await? else {
                return Ok(());
            };
            match self.remove_secret(plugin, TOKENS, revision).await {
                Ok(()) => break tokens,
                Err(PortFailure::Refused(PortRefusal::Conflict)) => continue,
                Err(failure) => return Err(failure),
            }
        };
        self.changed(plugin, None);
        if let Err(failure) = self.revoke(plugin, oauth, &tokens, cancel).await {
            // The tokens are forgotten here; the vendor lets them expire.
            tracing::warn!(plugin = self.plugin_id(plugin), %failure, "the vendor did not revoke a sign-in");
        }
        Ok(())
    }

    async fn revoke(
        &self,
        plugin: usize,
        oauth: &OAuth,
        tokens: &Tokens,
        cancel: &CancellationToken,
    ) -> Result<(), PortFailure> {
        let metadata = self.metadata(plugin, oauth, cancel).await?;
        let Some(endpoint) = metadata.revocation_endpoint else {
            return Ok(());
        };
        let revoked = [
            ("refresh_token", tokens.refresh_token.as_ref()),
            ("access_token", Some(&tokens.access_token)),
        ];
        for (hint, token) in revoked {
            let Some(token) = token else { continue };
            let form = form_body(&[
                ("token", token.expose()),
                ("token_type_hint", hint),
                ("client_id", tokens.client_id.as_str()),
            ]);
            let request =
                WebRequest::new(WebMethod::Post, endpoint.clone(), VENDOR_DEADLINE, VENDOR_BODY_BYTES)
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(form);
            let response = self.vendor(plugin, request, cancel).await?;
            if !(200..300).contains(&response.status) {
                return Err(vendor(format!(
                    "the revocation answered HTTP {}",
                    response.status
                )));
            }
        }
        Ok(())
    }

    /// Sends `form` to the token endpoint.
    async fn token_endpoint(
        &self,
        plugin: usize,
        metadata: &Metadata,
        form: &[(&str, &str)],
        cancel: &CancellationToken,
    ) -> Result<Exchanged, PortFailure> {
        let request = WebRequest::new(
            WebMethod::Post,
            metadata.token_endpoint.clone(),
            VENDOR_DEADLINE,
            VENDOR_BODY_BYTES,
        )
        .header("content-type", "application/x-www-form-urlencoded")
        .header("accept", "application/json")
        .body(form_body(form));
        let response = self.vendor(plugin, request, cancel).await?;
        match response.status {
            200..300 => Ok(Exchanged::Issued(decoded("token response", &response)?)),
            400 | 401 => {
                let error = decoded::<Refusal>("token refusal", &response)
                    .map(|refusal| refusal.error)
                    .unwrap_or_default();
                Ok(Exchanged::Refused {
                    unknown_client: error == "invalid_client",
                })
            }
            status => Err(vendor(format!("the token endpoint answered HTTP {status}"))),
        }
    }

    /// The tokens `issued` gives, keeping `refresh_token` when the vendor
    /// sent no new one.
    fn tokens(&self, client_id: String, issued: Issued, refresh_token: Option<Secret>) -> Tokens {
        let expires_at = issued.expires_in.0.map(|lifetime| {
            let lifetime = i64::try_from(lifetime.as_millis()).unwrap_or(i64::MAX);
            self.product.now().as_millisecond().saturating_add(lifetime)
        });
        Tokens {
            client_id,
            access_token: issued.access_token,
            refresh_token: issued.refresh_token.or(refresh_token),
            expires_at,
        }
    }

    /// Whether `tokens`' access token expires within a minute.
    fn due(&self, tokens: &Tokens) -> bool {
        let Some(expires_at) = tokens.expires_at else {
            return false;
        };
        let renew_from = self.product.now().to_jiff() + RENEW_WITHIN;
        expires_at <= renew_from.as_millisecond()
    }

    /// The user's tokens for plugin `plugin`, with their revision.
    async fn stored_tokens(&self, plugin: usize) -> Result<Option<(Tokens, u64)>, PortFailure> {
        let Some(StoredSecret { value, revision }) = self.read_secret(plugin, TOKENS).await? else {
            return Ok(None);
        };
        let tokens = decode_secret(&value).map_err(|error| {
            PortFailure::Port(PortError::Failed(format!(
                "the stored sign-in of the plugin \"{}\" is corrupt: {error}",
                self.plugin_id(plugin)
            )))
        })?;
        Ok(Some((tokens, revision)))
    }

    /// Forgets the tokens at `revision`, which the vendor will not renew,
    /// and answers that the user is signed out.
    async fn signed_out<T>(&self, plugin: usize, revision: u64) -> Result<T, PortFailure> {
        match self.remove_secret(plugin, TOKENS, revision).await {
            // A conflict is another worker's newer sign-in, which stays.
            Ok(()) | Err(PortFailure::Refused(PortRefusal::Conflict)) => {}
            Err(failure) => return Err(failure),
        }
        self.changed(plugin, None);
        Err(PortFailure::Refused(PortRefusal::SignedOut))
    }
}

/// How long ago `then` was at `now`.
fn elapsed(now: Timestamp, then: Timestamp) -> SignedDuration {
    now.to_jiff().duration_since(then.to_jiff())
}

/// 32 random bytes in base64url: a PKCE verifier (RFC 7636 § 4.1) or a
/// `state`.
fn random_text() -> String {
    URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>())
}

fn form_body(pairs: &[(&str, &str)]) -> String {
    url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(pairs)
        .finish()
}

fn sealed(tokens: &Tokens) -> String {
    serde_json::to_string(tokens).expect("tokens encode as JSON")
}

/// The authorization server's JSON answer `what` as `T`; a failure names
/// the field, never a value, since the answer may hold tokens.
fn decoded<T: DeserializeOwned>(what: &str, response: &WebResponse) -> Result<T, PortFailure> {
    let text = std::str::from_utf8(response.body.as_bytes())
        .map_err(|_| vendor(format!("the authorization server's {what} is not text")))?;
    decode_secret(text).map_err(|error| {
        vendor(format!("the authorization server's {what} is {error}"))
    })
}

fn vendor(message: String) -> PortFailure {
    PortFailure::Refused(PortRefusal::Web {
        failure: WebFailure::Vendor { message },
    })
}

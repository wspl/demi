//! Push notifications: each session's subscription, and the delivery of a
//! message to the subscriptions it goes to.
//!
//! - A subscription is kept once its endpoint's host resolves to public
//!   addresses.
//! - Each message is encrypted for its browser and signed with the backend's
//!   VAPID key (RFC 8291, RFC 8292) by `web-push-native`.
//! - A subscription whose push service answers 404 or 410 is gone and removed.
//!
//! The messages leave through a [`PushTransport`]: the internet's push
//! services, reached only at public addresses and checked as each connection is
//! made, or a test's stand-in. The backend gives the module the key. The module
//! derives none.
//!
//! A subscription serves one session, so a push queued for an account before it
//! signed out is never shown after another signs in: the browser drops what
//! comes for an ended subscription. A send follows no redirect, so a
//! subscription cannot make the backend call into its own network.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use demi_backend_database::StorageError;
use demi_backend_database::control::ControlService;
use demi_backend_database::push::{PutPush, StoredPushSubscription};
use demi_web_api_protocol::ids::UserId;
use demi_web_api_protocol::push::{PushEndpoint, PushMessage, PushSubscriptionBody};
use demi_web_api_protocol::sessions::SessionId;
use futures_util::future::{BoxFuture, join_all};
use reqwest::StatusCode;
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use url::Host;
use web_push_native::jwt_simple::algorithms::{ECDSAP256PublicKeyLike as _, ES256KeyPair};
use web_push_native::{Auth, WebPushBuilder};

/// How long a push service keeps a message for a browser that is offline:
/// a day, after which a turn's end is old news.
const TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// How long one push service may take to answer, so a slow one never holds
/// the others' messages.
const SEND_TIMEOUT: Duration = Duration::from_secs(30);

/// The backend's VAPID key pair, from 32 bytes the executable derives.
pub struct PushKey(ES256KeyPair);

impl PushKey {
    /// The key of the P-256 scalar `bytes`; none in the vanishing case that
    /// they are not one (zero, or the group's order or above).
    pub fn new(bytes: [u8; 32]) -> Option<Self> {
        ES256KeyPair::from_bytes(&bytes).ok().map(Self)
    }

    /// The public key the page subscribes with, the uncompressed point in
    /// URL-safe base64 without padding.
    pub fn public_key(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.0.public_key().public_key().to_bytes_uncompressed())
    }
}

impl std::fmt::Debug for PushKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PushKey(..)")
    }
}

/// Why a subscription is refused.
#[derive(Debug, thiserror::Error)]
pub enum PushError {
    #[error("the endpoint's host must resolve to public addresses: {0}")]
    NotPublic(String),
    #[error("another session's subscription has this endpoint")]
    Taken,
    #[error(transparent)]
    Storage(#[from] StorageError),
}

/// Where the push messages leave the backend.
pub trait PushTransport: Send + Sync + 'static {
    /// Whether messages may go to `endpoint`: why not, when its host is not
    /// a public one.
    fn admit<'a>(&'a self, endpoint: &'a PushEndpoint) -> BoxFuture<'a, Result<(), String>>;

    /// Posts `request` to its push service; answers the status it answered.
    fn post(&self, request: http::Request<Vec<u8>>) -> BoxFuture<'_, Result<StatusCode, String>>;
}

/// The internet's push services, reached only at public addresses.
///
/// The client resolves each host with [`PublicResolver`], so a name that
/// resolves elsewhere by the time a message is sent is refused then. The
/// transport follows no redirect and uses no proxy.
pub struct PublicPushTransport {
    client: reqwest::Client,
}

impl PublicPushTransport {
    pub fn new() -> Result<Self, reqwest::Error> {
        let client = reqwest::Client::builder()
            .dns_resolver(PublicResolver)
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .timeout(SEND_TIMEOUT)
            .build()?;
        Ok(Self { client })
    }
}

impl PushTransport for PublicPushTransport {
    fn admit<'a>(&'a self, endpoint: &'a PushEndpoint) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            match endpoint.url().host() {
                Some(Host::Domain(name)) => public_addresses(name).await.map(drop),
                Some(Host::Ipv4(address)) => public_literal(IpAddr::V4(address)),
                Some(Host::Ipv6(address)) => public_literal(IpAddr::V6(address)),
                // A push endpoint has a host.
                None => Err("it has no host".to_owned()),
            }
        })
    }

    fn post(&self, request: http::Request<Vec<u8>>) -> BoxFuture<'_, Result<StatusCode, String>> {
        Box::pin(async move {
            // The client resolves names only: an address in the URL is
            // checked here.
            if let Some(host) = request.uri().host()
                && let Ok(address) = host.trim_start_matches('[').trim_end_matches(']').parse::<IpAddr>()
            {
                public_literal(address)?;
            }
            let request = reqwest::Request::try_from(request).map_err(|error| error.to_string())?;
            let response = self.client.execute(request).await.map_err(|error| error.to_string())?;
            Ok(response.status())
        })
    }
}

/// Resolves a push service's host to its public addresses, refusing a name
/// that has none.
struct PublicResolver;

impl Resolve for PublicResolver {
    fn resolve(&self, name: Name) -> Resolving {
        Box::pin(async move {
            let addresses = public_addresses(name.as_str()).await?;
            let addrs: Addrs = Box::new(addresses.into_iter());
            Ok(addrs)
        })
    }
}

/// The public addresses `name` resolves to; an error when it resolves to
/// none.
async fn public_addresses(name: &str) -> Result<Vec<SocketAddr>, String> {
    let resolved = tokio::net::lookup_host((name, 0))
        .await
        .map_err(|error| format!("{name} does not resolve: {error}"))?;
    let public: Vec<SocketAddr> = resolved.filter(|address| is_public(address.ip())).collect();
    if public.is_empty() {
        return Err(format!("{name} resolves to no public address"));
    }
    Ok(public)
}

fn public_literal(address: IpAddr) -> Result<(), String> {
    if is_public(address) {
        Ok(())
    } else {
        Err(format!("{address} is not a public address"))
    }
}

/// Whether `address` is on the public internet.
///
/// It is not when it is in one of these ranges, or is an IPv6 address that
/// carries an IPv4 address in one:
///
/// - Loopback.
/// - Private.
/// - Shared (CGNAT).
/// - Link-local, the cloud metadata service's included.
/// - Unique local.
/// - Multicast.
/// - Documentation.
/// - Benchmarking.
/// - Reserved.
pub fn is_public(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => is_public_v4(address),
        IpAddr::V6(address) => is_public_v6(address),
    }
}

fn is_public_v4(address: Ipv4Addr) -> bool {
    let [a, b, c, _] = address.octets();
    !(address.is_unspecified()
        || address.is_loopback()
        || address.is_private()
        || address.is_link_local()
        || address.is_multicast()
        || address.is_broadcast()
        || address.is_documentation()
        || a == 0
        || (a == 100 && b & 0xc0 == 64)
        || (a == 192 && b == 0 && c == 0)
        || (a == 198 && b & 0xfe == 18)
        || a >= 240)
}

fn is_public_v6(address: Ipv6Addr) -> bool {
    if let Some(v4) = address.to_ipv4_mapped() {
        return is_public_v4(v4);
    }
    let segments = address.segments();
    // NAT64 and 6to4 carry an IPv4 address.
    if segments[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
        let [_, _, _, _, _, _, high, low] = segments;
        return is_public_v4(Ipv4Addr::from(u32::from(high) << 16 | u32::from(low)));
    }
    if segments[0] == 0x2002 {
        return is_public_v4(Ipv4Addr::from(u32::from(segments[1]) << 16 | u32::from(segments[2])));
    }
    !(address.is_unspecified()
        || address.is_loopback()
        || address.is_multicast()
        || address.is_unique_local()
        || address.is_unicast_link_local()
        // IPv4-compatible addresses, the documentation range and Teredo.
        || segments[..6] == [0; 6]
        || segments[..2] == [0x2001, 0xdb8]
        || segments[..2] == [0x2001, 0])
}

/// The users' push subscriptions and the delivery to them.
#[derive(Clone)]
pub struct Pusher {
    control: ControlService,
    key: Arc<PushKey>,
    transport: Arc<dyn PushTransport>,
}

impl Pusher {
    pub fn new(control: ControlService, key: PushKey, transport: Arc<dyn PushTransport>) -> Self {
        Self {
            control,
            key: Arc::new(key),
            transport,
        }
    }

    /// The public key the page subscribes with.
    pub fn public_key(&self) -> String {
        self.key.public_key()
    }

    /// Keeps the browser's subscription for `user`'s session `session`,
    /// whose end removes it, once its endpoint's host is a public one and
    /// unless another live session holds it.
    pub async fn subscribe(&self, user: UserId, session: SessionId, body: PushSubscriptionBody) -> Result<(), PushError> {
        let PushSubscriptionBody { endpoint, keys, events } = body;
        self.transport.admit(&endpoint).await.map_err(PushError::NotPublic)?;
        let kept = self
            .control
            .put_push_subscription(
                user,
                StoredPushSubscription {
                    session,
                    endpoint,
                    keys,
                    events,
                },
            )
            .await?;
        match kept {
            PutPush::Kept => Ok(()),
            PutPush::Taken => Err(PushError::Taken),
        }
    }

    /// Removes the session's subscription.
    pub async fn unsubscribe(&self, session: SessionId) -> Result<(), StorageError> {
        self.control.delete_push_subscription(session).await
    }

    /// `user`'s subscriptions, one per session with notifications on that
    /// has not expired.
    pub async fn subscriptions(&self, user: UserId) -> Result<Vec<StoredPushSubscription>, StorageError> {
        self.control.push_subscriptions(user).await
    }

    /// Delivers `message` to each of `subscriptions` at once, and removes each
    /// one its push service says is gone.
    ///
    /// Each VAPID signature names `contact`, who sends: the backend's public
    /// address. A failure is logged: a notification is not worth more than one
    /// try.
    pub async fn deliver(
        &self,
        user: &UserId,
        message: &PushMessage,
        subscriptions: &[StoredPushSubscription],
        contact: &str,
    ) -> bool {
        let body = match serde_json::to_vec(message) {
            Ok(body) => body,
            Err(error) => {
                tracing::error!("a push message did not serialize: {error}");
                return false;
            }
        };
        join_all(
            subscriptions
                .iter()
                .map(|subscription| self.deliver_one(user, subscription, &body, contact)),
        )
        .await
        .contains(&true)
    }

    /// Sends `body` to the subscription; answers whether its push service
    /// said it is gone and it was removed.
    async fn deliver_one(&self, user: &UserId, subscription: &StoredPushSubscription, body: &[u8], contact: &str) -> bool {
        let endpoint = match subscription.endpoint.as_str().parse::<http::Uri>() {
            Ok(endpoint) => endpoint,
            Err(error) => {
                // A URL `PushEndpoint` parsed is a URI too.
                tracing::error!(%user, "a push endpoint is not a URI: {error}");
                return false;
            }
        };
        let request = WebPushBuilder::new(
            endpoint,
            *subscription.keys.p256dh.key(),
            Auth::clone_from_slice(subscription.keys.auth.bytes()),
        )
        .with_valid_duration(TTL)
        .with_vapid(&self.key.0, contact)
        .build(body.to_vec());
        let request = match request {
            Ok(mut request) => {
                // A turn's end is what the user waits for: deliver it now,
                // not when the device next wakes on its own.
                request
                    .headers_mut()
                    .insert("urgency", http::HeaderValue::from_static("high"));
                request
            }
            Err(error) => {
                tracing::error!(%user, "a push message was not encrypted: {error}");
                return false;
            }
        };
        match self.transport.post(request).await {
            Ok(status) if status.is_success() => false,
            Ok(StatusCode::NOT_FOUND | StatusCode::GONE) => {
                tracing::info!(%user, "a push subscription is gone, and is removed");
                match self
                    .control
                    .delete_gone_push_subscription(subscription.session.clone(), subscription.endpoint.clone())
                    .await
                {
                    Ok(()) => true,
                    Err(error) => {
                        tracing::warn!(%user, "a gone push subscription was not removed: {error}");
                        false
                    }
                }
            }
            Ok(status) => {
                tracing::warn!(%user, %status, "a push service refused a message");
                false
            }
            Err(error) => {
                tracing::warn!(%user, "a push message was not sent: {error}");
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::FutureExt as _;

    fn endpoint(text: &str) -> PushEndpoint {
        PushEndpoint::try_from(text.to_owned()).unwrap()
    }

    // Cost: a few name lookups of `localhost` and one loopback listener;
    // milliseconds.

    #[tokio::test]
    async fn an_endpoint_whose_host_is_not_public_is_refused_when_admitted_and_when_sent_to() {
        let transport = PublicPushTransport::new().unwrap();
        for refused in [
            "https://localhost/push",
            "https://127.0.0.1/push",
            "https://[::1]/push",
            "https://10.1.2.3/push",
            "https://172.16.0.1/push",
            "https://192.168.1.1/push",
            "https://169.254.169.254/latest/meta-data",
            "https://100.64.0.1/push",
            "https://0.0.0.0/push",
            "https://[fd00:ec2::254]/push",
            "https://[fe80::1]/push",
            "https://[::ffff:127.0.0.1]/push",
            "https://[64:ff9b::a00:1]/push",
            "https://[2002:7f00:1::]/push",
        ] {
            assert!(transport.admit(&endpoint(refused)).await.is_err(), "{refused}");
        }
        for public in ["https://8.8.8.8/push", "https://[2606:4700::1111]/push"] {
            assert_eq!(transport.admit(&endpoint(public)).await, Ok(()), "{public}");
        }

        // A name admitted earlier that resolves to the loopback by the time
        // a message goes is refused as the connection is made: nothing
        // reaches the listener.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        for url in [format!("https://localhost:{port}/push"), format!("https://127.0.0.1:{port}/push")] {
            let request = http::Request::post(url.as_str()).body(Vec::new()).unwrap();
            let sent = transport.post(request).await;
            assert!(sent.is_err(), "{url}: {sent:?}");
        }
        assert!(listener.accept().now_or_never().is_none(), "no connection was made");
    }
}

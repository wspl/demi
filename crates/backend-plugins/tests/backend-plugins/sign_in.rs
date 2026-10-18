//! A plugin's OAuth sign-in (`plugins.md` § OAuth sign-in), against a fake
//! authorization server: Demi registers once, a user's sign-in exchanges
//! its code with its PKCE verifier, the tokens renew at their revision and
//! spend each refresh token once, a renewal runs to its end when its
//! caller goes away, only a refused refresh token signs the user out, and
//! signing out revokes them.

use demi_backend_plugins::testing::{Authority, AuthorizationServer};
use demi_backend_plugins::{Callback, CallbackError, Registry, UserPlugins};
use demi_plugin_interface::PortRefusal;
use jiff::SignedDuration;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::support::{Backend, PUBLIC_URL, PortProbe, VENDOR, call, refusal, within};

struct SignIn {
    server: AuthorizationServer,
    backend: Backend,
    _authority: Authority,
}

impl SignIn {
    async fn start() -> Self {
        let authority = Authority::new();
        let server = AuthorizationServer::start(&authority).await;
        let registry = Registry::new(
            vec![PortProbe::new("probe", Some(server.issuer())).boxed()],
            |_| true,
        )
        .unwrap();
        let backend = Backend::start(registry, vec![authority.root()]).await;
        Self {
            server,
            backend,
            _authority: authority,
        }
    }

    /// Starts a sign-in of `plugins`' user and answers its authorization
    /// URL.
    async fn sign_in_url(&self, plugins: &UserPlugins) -> String {
        let started = call(plugins, "probe", "sign_in", json!({})).await.unwrap();
        started["url"].as_str().unwrap().to_owned()
    }

    /// What the browser brings back once the user approves at `url`.
    async fn approve(&self, url: &str) -> Callback {
        let back = self.server.approve(url).await;
        let pairs: std::collections::HashMap<_, _> = back.query_pairs().into_owned().collect();
        Callback {
            state: pairs.get("state").cloned(),
            code: pairs.get("code").cloned(),
            error: None,
        }
    }

    async fn signed_in(&self, plugins: &UserPlugins) {
        let url = self.sign_in_url(plugins).await;
        let back = self.approve(&url).await;
        assert_eq!(plugins.oauth_callback("probe", back, CancellationToken::new()).await.unwrap(), VENDOR);
    }

    fn advance(&self, by: SignedDuration) {
        self.backend.product.clock.advance(by);
    }
}

async fn token(plugins: &UserPlugins) -> Result<String, PortRefusal> {
    match call(plugins, "probe", "token", json!({})).await {
        Ok(answer) => Ok(answer["token"].as_str().unwrap().to_owned()),
        answer => Err(refusal(answer)),
    }
}

#[tokio::test(flavor = "local")]
async fn a_user_signs_in_once_registered_and_the_token_renews_spending_its_refresh_token_once() {
    within(async {
        let sign_in = SignIn::start().await;
        let alice = sign_in.backend.user("alice").await;
        let plugins = sign_in.backend.plugins(&alice);
        assert_eq!(token(&plugins).await, Err(PortRefusal::SignedOut));

        let url = sign_in.sign_in_url(&plugins).await;
        let authorize = Url::parse(&url).unwrap();
        let query: std::collections::HashMap<_, _> = authorize.query_pairs().into_owned().collect();
        assert_eq!(
            query["redirect_uri"],
            format!("{PUBLIC_URL}/api/plugins/probe/oauth/callback")
        );
        assert_eq!(query["code_challenge_method"], "S256");
        assert_eq!(query["resource"], "https://mcp.example.test/mcp");
        assert_eq!(query["scope"], "tools");
        let back = sign_in.approve(&url).await;
        assert_eq!(plugins.oauth_callback("probe", back, CancellationToken::new()).await.unwrap(), VENDOR);
        let first = token(&plugins).await.unwrap();
        assert!(first.starts_with("access-"), "{first}");
        // The plugin reads only the access token; its own secrets do not
        // include the sign-in.
        assert_eq!(
            call(&plugins, "probe", "read_secret", json!({ "name": "oauth" }))
                .await
                .unwrap(),
            Value::Null
        );

        // Another user's sign-in, on another shard, uses the same client.
        let bob = sign_in.backend.user("bob").await;
        sign_in.signed_in(&sign_in.backend.plugins(&bob)).await;
        assert_eq!(sign_in.server.registrations(), 1);

        // Still live in more than a minute: no renewal.
        sign_in.advance(SignedDuration::from_secs(3600 - 61));
        assert_eq!(token(&plugins).await.unwrap(), first);
        assert_eq!(sign_in.server.refreshes(), 0);

        // Expiring within a minute: two calls at once renew it once.
        sign_in.advance(SignedDuration::from_secs(2));
        sign_in.server.hold_refreshes(true);
        let released = async {
            sign_in.server.until_refreshes(1).await;
            sign_in.server.hold_refreshes(false);
        };
        let (one, two, ()) = tokio::join!(token(&plugins), token(&plugins), released);
        let renewed = one.unwrap();
        assert_ne!(renewed, first);
        assert_eq!(two.unwrap(), renewed);
        assert_eq!(sign_in.server.refreshes(), 1);
        assert_eq!(token(&plugins).await.unwrap(), renewed);

        // Signing out revokes the refresh and the access token, and forgets
        // them.
        call(&plugins, "probe", "sign_out", json!({})).await.unwrap();
        let revoked = sign_in.server.revoked();
        assert_eq!(revoked.len(), 2, "{revoked:?}");
        assert!(revoked[0].starts_with("refresh-") && revoked[1] == renewed, "{revoked:?}");
        assert_eq!(token(&plugins).await, Err(PortRefusal::SignedOut));
    })
    .await;
}

#[tokio::test(flavor = "local")]
async fn a_state_another_user_started_an_old_one_or_one_used_before_is_refused() {
    within(async {
        let sign_in = SignIn::start().await;
        let alice = sign_in.backend.user("alice").await;
        let bob = sign_in.backend.user("bob").await;
        let alices = sign_in.backend.plugins(&alice);
        let bobs = sign_in.backend.plugins(&bob);

        let url = sign_in.sign_in_url(&alices).await;
        let back = sign_in.approve(&url).await;
        assert!(matches!(
            bobs.oauth_callback("probe", back.clone(), CancellationToken::new()).await,
            Err(CallbackError::UnknownState)
        ));
        assert_eq!(token(&bobs).await, Err(PortRefusal::SignedOut));
        assert_eq!(alices.oauth_callback("probe", back.clone(), CancellationToken::new()).await.unwrap(), VENDOR);
        assert!(matches!(
            alices.oauth_callback("probe", back, CancellationToken::new()).await,
            Err(CallbackError::UnknownState)
        ));

        // A sign-in waits ten minutes for its callback.
        let url = sign_in.sign_in_url(&bobs).await;
        let back = sign_in.approve(&url).await;
        sign_in.advance(SignedDuration::from_mins(10));
        assert!(matches!(
            bobs.oauth_callback("probe", back, CancellationToken::new()).await,
            Err(CallbackError::UnknownState)
        ));
        assert_eq!(token(&bobs).await, Err(PortRefusal::SignedOut));

        // A user who declines at the vendor is not signed in.
        let url = sign_in.sign_in_url(&bobs).await;
        let mut back = sign_in.approve(&url).await;
        back.code = None;
        back.error = Some("access_denied".into());
        assert!(matches!(
            bobs.oauth_callback("probe", back, CancellationToken::new()).await,
            Err(CallbackError::Denied { .. })
        ));
        assert_eq!(token(&bobs).await, Err(PortRefusal::SignedOut));
    })
    .await;
}

#[tokio::test(flavor = "local")]
async fn a_refused_renewal_signs_the_user_out_and_a_forgotten_client_is_registered_again() {
    within(async {
        let sign_in = SignIn::start().await;
        let alice = sign_in.backend.user("alice").await;
        let plugins = sign_in.backend.plugins(&alice);
        sign_in.signed_in(&plugins).await;
        sign_in.server.refuse_refreshes();
        sign_in.advance(SignedDuration::from_hours(2));
        assert_eq!(token(&plugins).await, Err(PortRefusal::SignedOut));
        // The tokens are forgotten: no second refresh.
        assert_eq!(token(&plugins).await, Err(PortRefusal::SignedOut));
        assert_eq!(sign_in.server.refreshes(), 1);

        // The vendor forgets Demi's client: the renewal is refused with
        // `invalid_client`, which is no refusal of the refresh token, so the
        // sign-in stays; the next sign-in registers again.
        let bob = sign_in.backend.user("bob").await;
        let bobs = sign_in.backend.plugins(&bob);
        sign_in.signed_in(&bobs).await;
        sign_in.server.forget_clients();
        sign_in.advance(SignedDuration::from_hours(2));
        assert!(matches!(token(&bobs).await, Err(PortRefusal::Web { .. })));
        assert_eq!(sign_in.server.registrations(), 1);
        sign_in.signed_in(&bobs).await;
        assert_eq!(sign_in.server.registrations(), 2);
        assert!(token(&bobs).await.is_ok());
    })
    .await;
}

/// A user's shard is on one worker, and its renewals take turns; while a
/// user moves between workers, two shards may renew at once. The first to
/// store its tokens wins, and the other uses them.
#[tokio::test(flavor = "local")]
async fn a_renewal_that_finds_another_shards_new_tokens_uses_them() {
    within(async {
        let sign_in = SignIn::start().await;
        let alice = sign_in.backend.user("alice").await;
        let one = sign_in.backend.plugins(&alice);
        let two = sign_in.backend.plugins(&alice);
        sign_in.signed_in(&one).await;
        sign_in.advance(SignedDuration::from_hours(2));
        // Both renew the expiring tokens, and a vendor that does not rotate
        // refresh tokens grants both.
        sign_in.server.reuse_refresh_tokens();
        sign_in.server.hold_refreshes(true);
        let released = async {
            sign_in.server.until_refreshes(2).await;
            sign_in.server.hold_refreshes(false);
        };
        let (first, second, ()) = tokio::join!(token(&one), token(&two), released);
        let (first, second) = (first.unwrap(), second.unwrap());
        assert_eq!(first, second);
        assert_eq!(token(&one).await.unwrap(), first);
        assert_eq!(token(&two).await.unwrap(), first);
    })
    .await;
}

/// A page's connection drops while its call waits for the vendor to renew
/// the token, as when the Exa sign-in was lost on 2026-10-16: the renewal
/// runs to its end and stores the new tokens, which the next call uses
/// without renewing again with the spent refresh token.
#[tokio::test(flavor = "local")]
async fn a_renewal_whose_caller_went_away_still_stores_the_new_tokens() {
    within(async {
        let sign_in = SignIn::start().await;
        let alice = sign_in.backend.user("alice").await;
        let plugins = sign_in.backend.plugins(&alice);
        sign_in.signed_in(&plugins).await;
        let first = token(&plugins).await.unwrap();
        sign_in.advance(SignedDuration::from_hours(2));
        sign_in.server.hold_refreshes(true);
        tokio::select! {
            answer = token(&plugins) => panic!("a held renewal answered {answer:?}"),
            () = sign_in.server.until_refreshes(1) => {}
        }
        // The caller is gone; the vendor answers now.
        sign_in.server.hold_refreshes(false);
        let renewed = token(&plugins).await.unwrap();
        assert_ne!(renewed, first);
        assert_eq!(sign_in.server.refreshes(), 1);
    })
    .await;
}

/// Only `invalid_grant` says the refresh token is dead. Another refusal, a
/// body that is not OAuth's, or a server error keeps the sign-in, answers
/// that the vendor did not answer, and the next call renews again.
#[tokio::test(flavor = "local")]
async fn a_renewal_that_fails_without_invalid_grant_keeps_the_sign_in() {
    within(async {
        let sign_in = SignIn::start().await;
        let alice = sign_in.backend.user("alice").await;
        let plugins = sign_in.backend.plugins(&alice);
        sign_in.signed_in(&plugins).await;
        sign_in.advance(SignedDuration::from_hours(2));
        let failures = [
            (400, r#"{"error":"temporarily_unavailable"}"#),
            (400, "Bad Request"),
            (401, ""),
            (503, r#"{"error":"server_error"}"#),
            // `invalid_grant` that is not a 400 or 401 with a JSON object.
            (400, r#"["invalid_grant"]"#),
            (200, r#"{"error":"invalid_grant"}"#),
            (500, r#"{"error":"invalid_grant"}"#),
        ];
        for failure in failures {
            sign_in.server.fail_refreshes(Some(failure));
            let answer = token(&plugins).await;
            assert!(matches!(answer, Err(PortRefusal::Web { .. })), "{failure:?}: {answer:?}");
        }
        sign_in.server.fail_refreshes(None);
        assert!(token(&plugins).await.unwrap().starts_with("access-"));
        assert_eq!(sign_in.server.refreshes(), failures.len() as u32 + 1);
    })
    .await;
}

/// A vendor's refusal may echo what it was sent: neither the log nor the
/// failure the plugin sees repeats an error code OAuth does not define.
#[tokio::test(flavor = "local")]
async fn a_refusal_that_echoes_a_secret_names_no_more_than_an_oauth_code() {
    within(async {
        let log = Log::default();
        let _logging = demi_log_capture::capture(
            tracing_subscriber::fmt()
                .with_writer(log.clone())
                .with_ansi(false)
                .finish(),
        );
        let sign_in = SignIn::start().await;
        let alice = sign_in.backend.user("alice").await;
        let plugins = sign_in.backend.plugins(&alice);
        sign_in.signed_in(&plugins).await;
        sign_in.advance(SignedDuration::from_hours(2));
        let echo = r#"{"error":"bad token refresh-secret-1","error_description":"refresh-secret-1"}"#;
        sign_in.server.fail_refreshes(Some((400, echo)));
        let answer = token(&plugins).await;
        let Err(PortRefusal::Web { failure }) = &answer else {
            panic!("expected the vendor not to answer, got {answer:?}");
        };
        let failure = failure.to_string();
        assert!(failure.contains("HTTP 400, other"), "{failure}");
        let logged = log.text();
        assert!(logged.contains("status=400") && logged.contains("other"), "{logged}");
        assert!(!logged.contains("secret") && !failure.contains("secret"), "{logged}");
    })
    .await;
}

/// What the scenario logged, as text.
#[derive(Clone, Default)]
struct Log(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl Log {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl std::io::Write for Log {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Log {
    type Writer = Log;

    fn make_writer(&'a self) -> Log {
        self.clone()
    }
}

//! Passkeys at the edge (`web-api.md` § Account API, `backend.md`
//! § Authentication and ownership), against a software authenticator and
//! WebAuthn client of the kind a password manager is: adding one takes the
//! password, a sign-in names no email, a synced passkey signs in, a removed
//! one is named back as unknown, and a challenge lasts five minutes.

use demi_shared_types::Clock as _;
use demi_web_api_protocol::auth::{Identity, Role};
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::passkeys::{PasskeyAnswer, PasskeyList};
use demi_web_api_protocol::state::SyncEvent;
use jiff::SignedDuration;
use passkey::authenticator::{Authenticator, UiHint, UserCheck, UserValidationMethod};
use passkey::client::{Client, DefaultClientData};
use public_suffix::PublicSuffixList;
use passkey::crypto::rust_crypto::RustCryptoBackend;
use passkey::types::ctap2::{Aaguid, Ctap2Error};
use passkey::types::webauthn::{CredentialCreationOptions, CredentialRequestOptions};
use passkey::types::Passkey;
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use crate::support::{Answer, Harness, MASTER_PASSWORD, Session, TestBackend, session_from};

/// The public URL the scenarios' backends serve at, whose host name is the
/// relying party's id; their requests reach the listener's own address.
const PUBLIC: &str = "http://localhost:3999";

/// Bitwarden's AAGUID, which the backend names.
const BITWARDEN: [u8; 16] = [
    0xd5, 0x48, 0x82, 0x6e, 0x79, 0xb4, 0xdb, 0x40, 0xa3, 0xd8, 0x11, 0x11, 0x6f, 0x7e, 0x83, 0x49,
];

/// A user who is present and verified each time the authenticator asks, as
/// Touch ID answers.
struct Verified;

#[async_trait::async_trait]
impl UserValidationMethod for Verified {
    type PasskeyItem = Passkey;

    async fn check_user<'a>(
        &self,
        _hint: UiHint<'a, Passkey>,
        _presence: bool,
        _verification: bool,
    ) -> Result<UserCheck, Ctap2Error> {
        Ok(UserCheck {
            presence: true,
            verification: true,
        })
    }

    fn is_presence_enabled(&self) -> bool {
        true
    }

    fn is_verification_enabled(&self) -> Option<bool> {
        Some(true)
    }
}

/// A password manager holding one passkey, which it marks backed up, in a
/// browser showing a page at `origin`.
struct Manager {
    client: Client<Option<Passkey>, Verified, RustCryptoBackend, PublicSuffixList, ()>,
    origin: url::Url,
}

impl Manager {
    fn new(aaguid: [u8; 16]) -> Self {
        Self::at(aaguid, PUBLIC)
    }

    fn at(aaguid: [u8; 16], origin: &str) -> Self {
        let authenticator = Authenticator::new(Aaguid(aaguid), None, Verified, RustCryptoBackend);
        Self {
            client: Client::new(authenticator).allows_insecure_localhost(true),
            origin: origin.parse().unwrap(),
        }
    }

    /// The browser's new credential for the creation options `options`.
    async fn create(&mut self, options: &Value) -> Value {
        let request = CredentialCreationOptions {
            public_key: serde_json::from_value(options.clone()).unwrap(),
        };
        let created = self
            .client
            .register(&self.origin, request, DefaultClientData)
            .await
            .unwrap();
        serde_json::to_value(created).unwrap()
    }

    /// The browser's signed answer to the request options `options`.
    async fn sign(&mut self, options: &Value) -> Value {
        let request = CredentialRequestOptions {
            public_key: serde_json::from_value(options.clone()).unwrap(),
        };
        let signed = self
            .client
            .authenticate(&self.origin, request, DefaultClientData)
            .await
            .unwrap();
        serde_json::to_value(signed).unwrap()
    }
}

/// A harness whose backends offer passkeys.
fn harness() -> Harness {
    let mut harness = Harness::new();
    harness.public_url = Some(PUBLIC.parse().unwrap());
    harness
}

/// Asks to add a passkey with `password`.
async fn add_options(backend: &TestBackend, session: &Session, password: &str) -> Answer {
    backend
        .post("/api/auth/passkeys/options", Some(session), json!({ "password": password }))
        .await
}

/// Adds a passkey of `manager` to the account, with the password, from a
/// page whose browser sends `user_agent`.
async fn add(backend: &TestBackend, session: &Session, manager: &mut Manager, user_agent: &str) -> Answer {
    let options = add_options(backend, session, MASTER_PASSWORD).await;
    assert_eq!(options.status, StatusCode::OK, "{}", String::from_utf8_lossy(&options.body));
    let options: Value = options.json();
    let credential = manager.create(&options["options"]).await;
    let body = json!({ "challengeId": options["challengeId"], "credential": credential });
    crate::support::answer(
        backend
            .response(
                Method::POST,
                "/api/auth/passkeys",
                session,
                &[("content-type", "application/json"), ("user-agent", user_agent)],
                Some(body.to_string().into()),
            )
            .await,
    )
    .await
}

/// A sign-in's request options, which name no account.
async fn sign_in_options(backend: &TestBackend) -> Value {
    let options = backend.post("/api/auth/passkey/options", None, json!({})).await;
    assert_eq!(options.status, StatusCode::OK, "{}", String::from_utf8_lossy(&options.body));
    options.json()
}

/// Signs in with the passkey `manager` holds, without an email.
async fn sign_in(backend: &TestBackend, manager: &mut Manager) -> Answer {
    let options = sign_in_options(backend).await;
    let credential = manager.sign(&options["options"]).await;
    backend
        .post(
            "/api/auth/passkey",
            None,
            json!({ "challengeId": options["challengeId"], "credential": credential }),
        )
        .await
}

const SAFARI: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15";

#[tokio::test]
async fn a_passkey_added_with_the_password_signs_in_without_an_email() {
    let harness = harness();
    let (backend, master) = harness.start_set_up().await;
    let mut page = backend.sync(&master).await;
    let state = page.snapshot().await;
    assert_eq!(state.passkey_rp_id.as_deref(), Some("localhost"));
    assert!(state.passkeys.passkeys.is_empty());

    // A wrong password adds nothing and asks the browser for nothing.
    let wrong = add_options(&backend, &master, "not the password").await;
    assert_eq!(wrong.refusal(), (StatusCode::UNAUTHORIZED, ErrorCode::InvalidCredentials));

    // The browser is asked for a passkey a sign-in without an email finds,
    // which verifies its user, of this backend's host name, and none the
    // account has.
    let options: Value = add_options(&backend, &master, MASTER_PASSWORD).await.json();
    let selection = &options["options"]["authenticatorSelection"];
    assert_eq!(selection["residentKey"], "required");
    assert_eq!(selection["userVerification"], "required");
    assert_eq!(options["options"]["rp"]["id"], "localhost");
    assert_eq!(options["options"]["user"]["name"], master.user.email.as_str());

    // Bitwarden is named for its AAGUID; a passkey of a manager the backend
    // does not know, such as Apple's, which carry none, for the page's
    // browser and system.
    let mut bitwarden = Manager::new(BITWARDEN);
    let added = add(&backend, &master, &mut bitwarden, SAFARI).await;
    assert_eq!(added.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&added.body));
    let first = added.json::<PasskeyAnswer>().passkey;
    assert_eq!(first.name, "Bitwarden");
    // A password manager's passkey is backed up: it syncs.
    assert!(first.synced);
    assert_eq!(first.last_used_at, None);
    let mut keychain = Manager::new([0; 16]);
    let second = add(&backend, &master, &mut keychain, SAFARI).await.json::<PasskeyAnswer>().passkey;
    assert_eq!(second.name, "Safari on macOS");
    let changes = page
        .until(|event| matches!(event, SyncEvent::Passkeys { passkeys, .. } if passkeys.len() == 2))
        .await;
    let Some(SyncEvent::Passkeys { user_handle, .. }) = changes.last() else {
        unreachable!("the wait ends at the passkeys")
    };
    let listed = backend.get("/api/auth/passkeys", Some(&master)).await.json::<PasskeyList>();
    assert_eq!(listed.passkeys, vec![first.clone(), second]);
    assert_eq!(&listed.user_handle, user_handle);

    // Signing in names no email: the passkey names the account.
    backend.post("/api/auth/logout", Some(&master), json!({})).await;
    let signed_in = sign_in(&backend, &mut bitwarden).await;
    assert_eq!(signed_in.status, StatusCode::OK, "{}", String::from_utf8_lossy(&signed_in.body));
    assert_eq!(signed_in.json::<Identity>().user, master.user);
    let session = session_from(&signed_in);
    assert_eq!(backend.get("/api/auth/me", Some(&session)).await.status, StatusCode::OK);
    let used = backend.get("/api/auth/passkeys", Some(&session)).await.json::<PasskeyList>();
    assert_eq!(used.passkeys[0].last_used_at, Some(harness.clock.now()));
    assert!(used.passkeys[0].synced);

    // Another account sees none of them, and cannot rename or remove one.
    harness.add_user("bob@example.test", "bob's password", Role::User);
    let bob = backend.login("bob@example.test", "bob's password").await;
    let bobs = backend.get("/api/auth/passkeys", Some(&bob)).await.json::<PasskeyList>();
    assert!(bobs.passkeys.is_empty());
    assert_ne!(bobs.user_handle, listed.user_handle);
    let path = format!("/api/auth/passkeys/{}", first.id);
    let renamed = backend.patch(&path, &bob, json!({ "name": "Mine" })).await;
    assert_eq!(renamed.refusal(), (StatusCode::NOT_FOUND, ErrorCode::PasskeyNotFound));
    assert_eq!(backend.delete(&path, &bob).await.refusal(), (StatusCode::NOT_FOUND, ErrorCode::PasskeyNotFound));

    // Its owner renames it, trimmed.
    let renamed = backend.patch(&path, &session, json!({ "name": "  Work laptop " })).await;
    assert_eq!(renamed.status, StatusCode::NO_CONTENT);
    let listed = backend.get("/api/auth/passkeys", Some(&session)).await.json::<PasskeyList>();
    assert_eq!(listed.passkeys[0].name, "Work laptop");
    let empty = backend.patch(&path, &session, json!({ "name": "   " })).await;
    assert_eq!(empty.refusal().1, ErrorCode::InvalidBody);
}

#[tokio::test]
async fn a_removed_passkey_is_named_back_as_unknown() {
    let harness = harness();
    let (backend, master) = harness.start_set_up().await;
    let mut manager = Manager::new([0; 16]);
    let passkey = add(&backend, &master, &mut manager, SAFARI).await.json::<PasskeyAnswer>().passkey;
    let removed = backend
        .delete(&format!("/api/auth/passkeys/{}", passkey.id), &master)
        .await;
    assert_eq!(removed.status, StatusCode::NO_CONTENT);

    // The manager still offers it; the backend names it so the page can
    // tell the browser, and signs nobody in.
    let refused = sign_in(&backend, &mut manager).await;
    assert_eq!(refused.refusal(), (StatusCode::UNAUTHORIZED, ErrorCode::UnknownPasskey));
    assert_eq!(refused.error().credential_id, Some(passkey.id));
    assert!(refused.session_cookies().is_empty());
}

#[tokio::test]
async fn a_challenge_lasts_five_minutes_and_is_used_once() {
    let harness = harness();
    let (backend, master) = harness.start_set_up().await;
    let mut manager = Manager::new([0; 16]);
    add(&backend, &master, &mut manager, SAFARI).await;

    let options = sign_in_options(&backend).await;
    let credential = manager.sign(&options["options"]).await;
    harness.clock.advance(SignedDuration::from_mins(5));
    let body = json!({ "challengeId": options["challengeId"], "credential": credential });
    let expired = backend.post("/api/auth/passkey", None, body.clone()).await;
    assert_eq!(expired.refusal(), (StatusCode::UNAUTHORIZED, ErrorCode::PasskeyExpired));

    // A challenge signs in once: its answer sent again is refused.
    let options = sign_in_options(&backend).await;
    let credential = manager.sign(&options["options"]).await;
    let body = json!({ "challengeId": options["challengeId"], "credential": credential });
    assert_eq!(backend.post("/api/auth/passkey", None, body.clone()).await.status, StatusCode::OK);
    let again = backend.post("/api/auth/passkey", None, body).await;
    assert_eq!(again.refusal(), (StatusCode::UNAUTHORIZED, ErrorCode::PasskeyExpired));

    // An addition's challenge is its account's alone.
    harness.add_user("bob@example.test", "bob's password", Role::User);
    let bob = backend.login("bob@example.test", "bob's password").await;
    let options: Value = add_options(&backend, &master, MASTER_PASSWORD).await.json();
    let credential = Manager::new([0; 16]).create(&options["options"]).await;
    let stolen = backend
        .post(
            "/api/auth/passkeys",
            Some(&bob),
            json!({ "challengeId": options["challengeId"], "credential": credential }),
        )
        .await;
    assert_eq!(stolen.refusal(), (StatusCode::UNAUTHORIZED, ErrorCode::PasskeyExpired));
}

#[tokio::test]
async fn with_a_thousand_challenges_waiting_a_sign_in_waits_until_they_expire() {
    let harness = harness();
    let (backend, _master) = harness.start_set_up().await;
    {
        let control = harness.control_database();
        let expires = harness.clock.now().as_millisecond() + 60_000;
        for n in 0..1000 {
            control
                .execute(
                    "INSERT INTO passkey_challenges (id, kind, user_id, state, expires_at)
                     VALUES (?1, 'sign_in', NULL, '{}', ?2)",
                    rusqlite::params![format!("c{n}"), expires],
                )
                .unwrap();
        }
    }
    let refused = backend.post("/api/auth/passkey/options", None, json!({})).await;
    assert_eq!(refused.refusal(), (StatusCode::TOO_MANY_REQUESTS, ErrorCode::TooManyAttempts));
    harness.clock.advance(SignedDuration::from_mins(1));
    sign_in_options(&backend).await;
}

#[tokio::test]
async fn a_backend_at_an_ip_address_offers_no_passkeys() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let status = backend.get("/api/setup", None).await.json::<Value>();
    assert_eq!(status["passkeyRpId"], Value::Null);
    let refused = add_options(&backend, &master, MASTER_PASSWORD).await;
    assert_eq!(refused.refusal(), (StatusCode::CONFLICT, ErrorCode::PasskeysUnavailable));
    let refused = backend.post("/api/auth/passkey/options", None, json!({})).await;
    assert_eq!(refused.refusal(), (StatusCode::CONFLICT, ErrorCode::PasskeysUnavailable));
}

#[tokio::test]
async fn a_development_page_on_another_port_adds_a_passkey() {
    // The development server passes the page's requests on with the page's
    // own host and port, which the origin check admits.
    const PAGE: &str = "http://localhost:18934";
    let harness = harness();
    let (backend, master) = harness.start_set_up().await;
    let mut manager = Manager::at([0; 16], PAGE);
    let from_page = |method: Method, path: &str, body: Value| {
        reqwest::Client::new()
            .request(method, format!("{}{path}", backend.url))
            .header("origin", PAGE)
            .header("host", "localhost:18934")
            .header("content-type", "application/json")
            .header("cookie", &master.cookie)
            .body(body.to_string())
            .send()
    };
    let options = crate::support::answer(
        from_page(Method::POST, "/api/auth/passkeys/options", json!({ "password": MASTER_PASSWORD }))
            .await
            .unwrap(),
    )
    .await
    .json::<Value>();
    let credential = manager.create(&options["options"]).await;
    let added = crate::support::answer(
        from_page(
            Method::POST,
            "/api/auth/passkeys",
            json!({ "challengeId": options["challengeId"], "credential": credential }),
        )
        .await
        .unwrap(),
    )
    .await;
    assert_eq!(added.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&added.body));

    // A page of another site is refused before any check of its passkey.
    let options = sign_in_options(&backend).await;
    let credential = manager.sign(&options["options"]).await;
    let elsewhere = crate::support::answer(
        reqwest::Client::new()
            .post(format!("{}/api/auth/passkey", backend.url))
            .header("origin", "http://localhost:4000")
            .header("content-type", "application/json")
            .body(json!({ "challengeId": options["challengeId"], "credential": credential }).to_string())
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(elsewhere.refusal(), (StatusCode::FORBIDDEN, ErrorCode::ForbiddenOrigin));
}

//! A subscription entry's accounts (`web-api.md` § Subscription accounts,
//! `providers.md` § Login and publication, `usage-and-quota.md` § Vendor
//! quota): setup tokens sealed and never returned, explicit selection,
//! device logins published once, held entries, cancellation and expiry, and
//! an account's quota snapshot through a probe and a restart. The families
//! are scripted.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use demi_backend::{FamilyRegistry, LoginTiming};
use demi_core::{QuotaWindow, SnapshotSource};
use demi_provider::quota::ProbeCost;
use demi_provider::testing::{MockResponse, MockVendor};
use demi_web_api::auth::Role;
use demi_web_api::error::ErrorCode;
use demi_web_api::providers::{
    Accounts, ActiveAccount, AddedAccount, CredentialKind, LoginAnswer, LoginStarted, LoginState,
    ProbeCost as ProbeCostDto, ProviderAnswer, ProviderDetails, ProviderDto, Providers, QuotaAnswer, QuotaCapability,
    VendorCatalog,
};
use demi_web_api::settings::InstanceMode;
use reqwest::StatusCode;
use serde_json::json;

use crate::families::{Directory, LoginScript, QuotaScript, ScriptedSubscription};
use crate::support::{Harness, Session, TestBackend};

pub(crate) struct Scripts {
    pub(crate) login: Arc<LoginScript>,
    pub(crate) families: FamilyRegistry,
}

/// A registry whose `claude-code` and `device` families are scripted
/// subscriptions sharing one login script, with `cost` for their probes.
pub(crate) fn scripts(cost: Option<ProbeCost>) -> Scripts {
    let login = Arc::new(LoginScript::default());
    let window = QuotaWindow {
        id: "weekly".into(),
        label: "Weekly".into(),
        used_percent: Some(40.0),
        used: None,
        limit: None,
        unit: None,
        resets_at: None,
        severity: None,
        scope: None,
    };
    let mut quota = QuotaScript::free(vec![window]);
    quota.cost = cost;
    let quota = Arc::new(quota);
    let family = || ScriptedSubscription {
        login: login.clone(),
        quota: quota.clone(),
        directory: Arc::new(Directory::default()),
    };
    let families = FamilyRegistry::builtin()
        .with("claude-code", family())
        .with("device", family());
    Scripts { login, families }
}

async fn start_login(backend: &TestBackend, session: &Session, path: &str, body: serde_json::Value) -> String {
    let answer = backend.post(path, Some(session), body).await;
    assert_eq!(
        answer.status,
        StatusCode::ACCEPTED,
        "{}",
        String::from_utf8_lossy(&answer.body)
    );
    let started = serde_json::from_slice::<serde_json::Value>(&answer.body).unwrap();
    assert_eq!(started["login"]["status"], "pending");
    answer.json::<LoginStarted>().login.id.into_string()
}

async fn login_state(backend: &TestBackend, session: &Session, id: &str) -> LoginState {
    backend
        .get(&format!("/api/providers/subscription-login/{id}"), Some(session))
        .await
        .json::<LoginAnswer>()
        .login
}

/// The login's state once `settled` says it has moved on, polled for up to
/// five seconds.
async fn awaited(
    backend: &TestBackend,
    session: &Session,
    id: &str,
    settled: impl Fn(&LoginState) -> bool,
) -> LoginState {
    for _ in 0..500 {
        let state = login_state(backend, session, id).await;
        if settled(&state) {
            return state;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("login {id} did not settle");
}

fn ended(state: &LoginState) -> bool {
    !matches!(state, LoginState::Pending { .. })
}

#[tokio::test]
async fn a_setup_token_becomes_a_sealed_account_that_no_answer_returns() {
    let scripts = scripts(Some(ProbeCost::Free));
    let harness = Harness::new().with_families(scripts.families);
    let (backend, master) = harness.start_set_up().await;
    let created = backend
        .post(
            "/api/providers/setup-token",
            Some(&master),
            json!({ "token": " fixture-token-a ", "label": "Claude" }),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED);
    let entry = created.json::<ProviderAnswer>().provider;
    assert_eq!(
        (entry.kind, entry.provider_type.as_str()),
        (CredentialKind::Subscription, "claude-code")
    );
    let again = backend
        .post(
            "/api/providers/setup-token",
            Some(&master),
            json!({ "token": "fixture-token-c", "label": "Again" }),
        )
        .await;
    assert_eq!(again.refusal(), (StatusCode::CONFLICT, ErrorCode::ProviderExists));

    let path = format!("/api/providers/{}/accounts", entry.id);
    let added = backend
        .post(&path, Some(&master), json!({ "token": "fixture-token-b" }))
        .await;
    assert_eq!(added.status, StatusCode::CREATED);
    let second = added.json::<AddedAccount>().account;
    let refused = backend
        .post(&path, Some(&master), json!({ "token": "bad-token-1" }))
        .await;
    assert_eq!(
        refused.refusal(),
        (StatusCode::BAD_REQUEST, ErrorCode::TokenImportFailed)
    );
    assert!(!String::from_utf8_lossy(&refused.body).contains("bad-token-1"));

    let listed = backend.get(&path, Some(&master)).await;
    assert!(!String::from_utf8_lossy(&listed.body).contains("fixture-token"));
    let accounts = listed.json::<Accounts>();
    assert_eq!(accounts.accounts.len(), 2);
    let first = accounts.active.clone().unwrap();
    assert_ne!(first.as_str(), second.id);
    let secrets: Vec<Vec<u8>> = {
        let database = harness.control_database();
        let mut statement = database.prepare("SELECT secret FROM provider_credentials").unwrap();
        statement
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    assert_eq!(secrets.len(), 2);
    assert!(
        secrets
            .iter()
            .all(|secret| !secret.windows(13).any(|window| window == b"fixture-token"))
    );

    // Selecting is explicit, and the active account cannot be removed.
    let active = format!("{path}/{first}");
    assert_eq!(
        backend.delete(&active, &master).await.refusal(),
        (StatusCode::CONFLICT, ErrorCode::ActiveAccount)
    );
    let switched = backend
        .put(&format!("{path}/active"), &master, json!({ "credentialId": second.id }))
        .await;
    assert_eq!(switched.json::<ActiveAccount>().active.as_str(), second.id);
    assert_eq!(backend.delete(&active, &master).await.status, StatusCode::NO_CONTENT);
    let missing = backend
        .put(
            &format!("{path}/active"),
            &master,
            json!({ "credentialId": "cred-missing" }),
        )
        .await;
    assert_eq!(missing.refusal(), (StatusCode::NOT_FOUND, ErrorCode::AccountNotFound));
    let status = backend
        .get(&format!("/api/providers/{}/status", entry.id), Some(&master))
        .await;
    assert!(!String::from_utf8_lossy(&status.body).contains("fixture-token"));
    let status = status.json::<ProviderDetails>();
    assert_eq!(
        (status.accounts.len(), status.active.as_ref().map(|id| id.as_str())),
        (1, Some(second.id.as_str()))
    );

    // Someone who only infers with a shared instance's entry sees whether it
    // works, not whose account it is.
    harness.add_user("reader@example.test", "reader-pass-1", Role::User);
    let reader = backend.login("reader@example.test", "reader-pass-1").await;
    let refused = backend
        .post(&path, Some(&reader), json!({ "token": "not-allowed" }))
        .await;
    assert_eq!(refused.refusal(), (StatusCode::FORBIDDEN, ErrorCode::Forbidden));
    let hidden = backend.get(&path, Some(&reader)).await.json::<Accounts>();
    assert_eq!((hidden.accounts.len(), hidden.active), (0, None));
    let seen = backend
        .get(&format!("/api/providers/{}/status", entry.id), Some(&reader))
        .await
        .json::<ProviderDetails>();
    assert_eq!(
        (seen.accounts.len(), seen.active.clone(), seen.quota.clone()),
        (0, None, None)
    );
    assert_eq!(seen.auth, demi_core::AuthState::Authenticated { account_label: None });
    backend.close().await;
}

#[tokio::test]
async fn concurrent_device_logins_publish_one_entry_and_the_other_stores_nothing() {
    let scripts = scripts(Some(ProbeCost::Free));
    // The catalog this scenario reads lists the vendors of a models.dev
    // document that names none.
    let vendor = MockVendor::start().await;
    vendor.respond_at("/api.json", MockResponse::status(200).chunk("{}"));
    let harness = Harness::new()
        .with_families(scripts.families)
        .with_models_dev(vendor.url("/api.json"));
    let (backend, master) = harness.start_set_up().await;
    let login = "/api/providers/subscription-login";
    for (body, refusal) in [
        (
            json!({ "providerType": "nope" }),
            (StatusCode::BAD_REQUEST, ErrorCode::UnknownProviderType),
        ),
        (
            json!({ "providerType": "anthropic" }),
            (StatusCode::BAD_REQUEST, ErrorCode::NoLoginFlow),
        ),
        (
            json!({ "type": "device" }),
            (StatusCode::BAD_REQUEST, ErrorCode::InvalidBody),
        ),
    ] {
        assert_eq!(
            backend.post(login, Some(&master), body.clone()).await.refusal(),
            refusal,
            "{body}"
        );
    }
    let first = start_login(
        &backend,
        &master,
        login,
        json!({ "providerType": "device", "label": "Work" }),
    )
    .await;
    let second = start_login(
        &backend,
        &master,
        login,
        json!({ "providerType": "device", "label": "Competing" }),
    )
    .await;
    let pending = awaited(&backend, &master, &first, |state| {
        matches!(
            state,
            LoginState::Pending {
                verification_url: Some(_),
                ..
            }
        )
    })
    .await;
    assert_eq!(
        pending,
        LoginState::Pending {
            verification_url: Some("https://verify.example/device".into()),
            user_code: Some("ABCD-1234".into()),
            expires_at: None,
        }
    );

    scripts.login.approve(true);
    let outcomes = [
        awaited(&backend, &master, &first, ended).await,
        awaited(&backend, &master, &second, ended).await,
    ];
    let completed: Vec<&LoginState> = outcomes
        .iter()
        .filter(|state| matches!(state, LoginState::Completed { .. }))
        .collect();
    assert_eq!(completed.len(), 1, "{outcomes:?}");
    let LoginState::Completed {
        provider_id,
        credential_id,
    } = completed[0]
    else {
        unreachable!();
    };
    assert!(outcomes.iter().any(|state| matches!(
        state,
        LoginState::Failed { message } if message == "This scope already has a device subscription"
    )));
    let providers = backend
        .get("/api/providers", Some(&master))
        .await
        .json::<Providers>()
        .providers;
    assert_eq!(
        providers.iter().map(|provider| &provider.id).collect::<Vec<_>>(),
        [provider_id]
    );
    let accounts = backend
        .get(&format!("/api/providers/{provider_id}/accounts"), Some(&master))
        .await
        .json::<Accounts>();
    assert_eq!(
        accounts.accounts.iter().map(|account| &account.id).collect::<Vec<_>>(),
        [credential_id.as_str()]
    );
    assert_eq!(accounts.active.as_ref(), Some(credential_id));
    let stored: (i64, Vec<u8>) = harness
        .control_database()
        .query_row("SELECT COUNT(*), MAX(secret) FROM provider_credentials", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .unwrap();
    assert_eq!(stored.0, 1);
    assert!(!stored.1.windows(12).any(|window| window == b"login-secret"));

    // One subscription entry per owner and family.
    let offered = backend.get("/api/providers/catalog", Some(&master)).await;
    assert!(
        offered
            .json::<VendorCatalog>()
            .subscriptions
            .iter()
            .any(|family| family.provider_type == "device" && family.configured)
    );
    let again = backend
        .post(login, Some(&master), json!({ "providerType": "device" }))
        .await;
    assert_eq!(again.refusal(), (StatusCode::CONFLICT, ErrorCode::ProviderExists));
    // A subscription entry takes a new label and nothing else.
    let path = format!("/api/providers/{provider_id}");
    let relabelled = backend.patch(&path, &master, json!({ "label": "Personal" })).await;
    assert_eq!(relabelled.json::<ProviderAnswer>().provider.label, "Personal");
    let rekeyed = backend.patch(&path, &master, json!({ "apiKey": "k" })).await;
    assert_eq!(
        rekeyed.refusal(),
        (StatusCode::BAD_REQUEST, ErrorCode::SubscriptionOnly)
    );
    assert_eq!(backend.delete(&path, &master).await.status, StatusCode::NO_CONTENT);
    let left: i64 = harness
        .control_database()
        .query_row("SELECT COUNT(*) FROM provider_credentials", [], |row| row.get(0))
        .unwrap();
    assert_eq!(left, 0);
    backend.close().await;
}

/// An entry of the `device` family with one account, from a login.
pub(crate) async fn device_entry(backend: &TestBackend, session: &Session, scripts: &Scripts) -> ProviderDto {
    scripts.login.approve(true);
    let id = start_login(
        backend,
        session,
        "/api/providers/subscription-login",
        json!({ "providerType": "device" }),
    )
    .await;
    let state = awaited(backend, session, &id, ended).await;
    assert!(matches!(state, LoginState::Completed { .. }), "{state:?}");
    backend
        .get("/api/providers", Some(session))
        .await
        .json::<Providers>()
        .providers
        .remove(0)
}

#[tokio::test]
async fn a_login_into_an_entry_holds_it_until_it_ends_and_cancelling_stops_it_at_once() {
    let scripts = scripts(Some(ProbeCost::Free));
    let harness = Harness::new().with_families(scripts.families.clone());
    let (backend, master) = harness.start_set_up().await;
    let entry = device_entry(&backend, &master, &scripts).await;
    scripts.login.approve(false);
    let path = format!("/api/providers/{}", entry.id);
    let id = start_login(&backend, &master, &format!("{path}/accounts/login"), json!({})).await;
    for busy in [
        backend.patch(&path, &master, json!({ "label": "Busy" })).await,
        backend.delete(&path, &master).await,
    ] {
        assert_eq!(busy.refusal(), (StatusCode::CONFLICT, ErrorCode::ProviderBusy));
    }
    let cancelled = backend
        .delete(&format!("/api/providers/subscription-login/{id}"), &master)
        .await;
    assert_eq!(cancelled.status, StatusCode::NO_CONTENT);
    assert_eq!(scripts.login.cancelled.load(Ordering::SeqCst), 1);
    assert_eq!(
        login_state(&backend, &master, &id).await,
        LoginState::Failed {
            message: "The login was cancelled".into()
        }
    );
    assert_eq!(
        backend.patch(&path, &master, json!({ "label": "Ready" })).await.status,
        StatusCode::OK
    );
    let unknown = backend
        .delete("/api/providers/subscription-login/no-such-login", &master)
        .await;
    assert_eq!(unknown.refusal(), (StatusCode::NOT_FOUND, ErrorCode::LoginNotFound));
    backend.close().await;
}

#[tokio::test]
async fn a_login_expires_and_its_result_goes_after_the_retention() {
    let scripts = scripts(Some(ProbeCost::Free));
    let timing = LoginTiming {
        lifetime: Duration::from_millis(100),
        retention: Duration::from_millis(300),
    };
    let harness = Harness::new().with_families(scripts.families).with_logins(timing);
    let (backend, master) = harness.start_set_up().await;
    let id = start_login(
        &backend,
        &master,
        "/api/providers/subscription-login",
        json!({ "providerType": "device" }),
    )
    .await;
    let expired = awaited(&backend, &master, &id, ended).await;
    assert_eq!(
        expired,
        LoginState::Failed {
            message: "The login expired".into()
        }
    );
    assert_eq!(
        scripts.login.cancelled.load(Ordering::SeqCst),
        1,
        "an expired login stops its flow"
    );
    assert!(
        backend
            .get("/api/providers", Some(&master))
            .await
            .json::<Providers>()
            .providers
            .is_empty()
    );
    tokio::time::sleep(timing.retention).await;
    let gone = backend
        .get(&format!("/api/providers/subscription-login/{id}"), Some(&master))
        .await;
    assert_eq!(gone.refusal(), (StatusCode::NOT_FOUND, ErrorCode::LoginNotFound));
    backend.close().await;
}

#[tokio::test]
async fn a_free_probe_fills_the_accounts_snapshot_which_outlives_a_restart() {
    let scripts = scripts(Some(ProbeCost::Free));
    let harness = Harness::new().with_families(scripts.families.clone());
    let (backend, master) = harness.start_set_up().await;
    let entry = device_entry(&backend, &master, &scripts).await;
    let path = format!("/api/providers/{}", entry.id);
    let status = backend
        .get(&format!("{path}/status"), Some(&master))
        .await
        .json::<ProviderDetails>();
    assert_eq!(
        (status.quota.clone(), status.quota_capability),
        (
            None,
            QuotaCapability::Supported {
                probe: Some(ProbeCostDto::Free)
            }
        )
    );

    // A body is optional: without one, the active account is probed.
    let probed = backend
        .send(
            reqwest::Method::POST,
            &format!("{path}/quota"),
            Some(&master.cookie),
            None,
        )
        .await;
    let snapshot = probed.json::<QuotaAnswer>().quota.unwrap();
    assert_eq!(
        (snapshot.source, snapshot.account_label.as_deref()),
        (SnapshotSource::Probe, Some("device@example.test"))
    );
    assert_eq!(snapshot.windows[0].used_percent, Some(40.0));
    let status = backend
        .get(&format!("{path}/status"), Some(&master))
        .await
        .json::<ProviderDetails>();
    assert_eq!(
        (status.quota.as_ref(), status.accounts[0].quota.as_ref()),
        (Some(&snapshot), Some(&snapshot))
    );
    let missing = backend
        .post(
            &format!("{path}/quota"),
            Some(&master),
            json!({ "credentialId": "cred-missing" }),
        )
        .await;
    assert_eq!(missing.refusal(), (StatusCode::NOT_FOUND, ErrorCode::AccountNotFound));

    // The snapshot is the account's record, which a restart reads back.
    backend.close().await;
    let backend = harness.start().await;
    let restored = backend
        .get(&format!("{path}/status"), Some(&master))
        .await
        .json::<ProviderDetails>();
    assert_eq!(restored.quota, Some(snapshot));
    backend.close().await;
}

#[tokio::test]
async fn a_probe_that_would_spend_inference_is_refused_and_an_api_key_entry_has_no_quota() {
    let scripts = scripts(Some(ProbeCost::Inference));
    let harness = Harness::new()
        .with_families(scripts.families.clone())
        .with_mode(InstanceMode::Isolated);
    let (backend, master) = harness.start_set_up().await;
    let entry = device_entry(&backend, &master, &scripts).await;
    let refused = backend
        .post(&format!("/api/providers/{}/quota", entry.id), Some(&master), json!({}))
        .await;
    assert_eq!(
        refused.refusal(),
        (StatusCode::CONFLICT, ErrorCode::QuotaRequiresInference)
    );
    let keyed = backend
        .post(
            "/api/providers",
            Some(&master),
            json!({ "source": "custom", "providerType": "anthropic", "label": "Work", "apiKey": "k" }),
        )
        .await
        .json::<ProviderAnswer>()
        .provider;
    let quota = backend
        .post(&format!("/api/providers/{}/quota", keyed.id), Some(&master), json!({}))
        .await;
    assert_eq!(quota.json::<QuotaAnswer>().quota, None);
    let status = backend
        .get(&format!("/api/providers/{}/status", keyed.id), Some(&master))
        .await
        .json::<ProviderDetails>();
    assert_eq!(
        (status.quota_capability, status.accounts.len()),
        (QuotaCapability::None {}, 0)
    );
    backend.close().await;
}

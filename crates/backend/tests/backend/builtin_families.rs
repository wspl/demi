//! The built-in provider families (`providers.md` § Families, vendors and
//! endpoints), built without an entry: the subscription families log in by
//! device, and each stands for the account an entry binds it to.

use demi_backend_providers::llm::families::SubscriptionArgs;
use demi_provider_common::credentials::{
    AccountMeta, AccountsCapability, CredentialPool, LoginKind, MemoryCredentialPool,
};
use demi_provider_common::models_dev::ModelsDevClient;
use demi_provider_common::testing::{FixedClock, MockResponse, MockVendor, guarded, jwt};
use demi_shared_types::{AuthState, Clock, QuotaSnapshot, SnapshotSource};
use serde_json::json;

use std::sync::Arc;

use demi_backend::families::{CodexFamily, builtin};
use demi_backend_providers::llm::families::{
    AccountBinding, FamilyArgs, FamilyCredential, ProviderFamily,
};
use demi_provider_common::Provider;
use demi_provider_common::quota::{MemorySnapshots, QuotaSnapshotStore};

const NOW: &str = "2026-09-18T14:00:00.000Z";

/// A provider of the built-in `family` over `pool`, for `account`. No
/// test here makes a request.
fn built(
    family: &str,
    pool: &MemoryCredentialPool,
    account: Option<AccountBinding>,
) -> Arc<dyn Provider> {
    builtin()
        .get(family)
        .unwrap()
        .provider(args(family, pool, account))
        .unwrap()
}

/// What a family builds a provider of `family` over `pool` with.
fn args(family: &str, pool: &MemoryCredentialPool, account: Option<AccountBinding>) -> FamilyArgs {
    let clock: Arc<dyn Clock> = Arc::new(FixedClock(NOW.parse().unwrap()));
    let url = ModelsDevClient::DEFAULT_URL.parse().unwrap();
    FamilyArgs {
        entry_id: "entry-1".into(),
        label: family.into(),
        credential: FamilyCredential::Subscription(SubscriptionArgs {
            pool: Arc::new(pool.clone()),
            account,
        }),
        http: reqwest::Client::new(),
        machine: None,
        clock: clock.clone(),
        models_dev: ModelsDevClient::new(reqwest::Client::new(), url, clock),
    }
}

#[tokio::test]
async fn every_codex_provider_of_a_backend_asks_with_the_one_version_the_family_read() {
    let registry = MockVendor::start().await;
    let record = json!({ "name": "@openai/codex", "version": "0.163.0" });
    registry.respond_at("/latest", MockResponse::status(200).chunk(record.to_string()));
    let family = CodexFamily::new(registry.url("/latest").parse().unwrap());
    let pool = MemoryCredentialPool::new();
    let first = family.provider(args("codex", &pool, None)).unwrap();
    let second = family.provider(args("codex", &pool, None)).unwrap();
    assert_eq!(second.catalog_revision().as_deref(), Some("0.162.1"));

    // A catalog read starts the read of the registry, then fails without a
    // request to Codex: the provider has no account.
    assert!(first.list_models().await.is_err());
    let read = async {
        while second.catalog_revision().as_deref() != Some("0.163.0") {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    };
    guarded("the version the first provider's read brings", read).await;
    assert!(second.list_models().await.is_err());
    assert_eq!(registry.requests().len(), 1);
}

/// An account whose snapshot says it was probed.
async fn account(pool: &MemoryCredentialPool, secret: serde_json::Value) -> AccountBinding {
    let meta = AccountMeta {
        id: "cred-1".into(),
        label: "user@example.com".into(),
        detail: None,
        updated_at: NOW.parse().unwrap(),
        source: "login:device".into(),
        identity_key: None,
        signed_out: false,
    };
    pool.write(meta, Some(secret.to_string())).await.unwrap();
    let quota = Arc::new(MemorySnapshots::new());
    quota.update(&mut |_| QuotaSnapshot {
        observed_at: NOW.parse().unwrap(),
        source: SnapshotSource::Probe,
        plan: None,
        account_label: Some("user@example.com".into()),
        windows: Vec::new(),
    });
    AccountBinding {
        credential_id: "cred-1".into(),
        quota,
    }
}

#[tokio::test]
async fn the_subscription_families_log_in_by_device_and_stand_for_their_bound_account() {
    let registry = builtin();
    let subscriptions: Vec<&str> = registry.subscriptions().collect();
    assert_eq!(subscriptions, ["claude-code", "codex", "grok-build"]);

    let codex = json!({
        "accessToken": jwt(&json!({ "exp": 1_900_000_000 })),
        "refreshToken": "refresh-1",
        "idToken": jwt(&json!({ "email": "user@example.com" })),
        "accountId": "acct-1",
        "lastRefresh": NOW,
    });
    let grok = json!({
        "accessToken": "session-token",
        "issuer": "https://auth.x.ai",
        "clientId": "client-1",
        "email": "user@example.com",
    });
    for (family, secret, name) in [("codex", codex, "Codex"), ("grok-build", grok, "Grok")] {
        // Built to log in: the device login, and no account yet.
        let staged = MemoryCredentialPool::new();
        let login = built(family, &staged, None);
        let device_login = AccountsCapability {
            login: Some(LoginKind::Device),
        };
        assert_eq!(
            login.accounts().unwrap().capability(),
            device_login,
            "{family}"
        );
        let unauthenticated = AuthState::Unauthenticated {
            message: Some(format!("No {name} account is signed in")),
        };
        assert_eq!(login.auth_status().await, unauthenticated, "{family}");

        // Built for an entry's account: that account's secret and quota.
        let pool = MemoryCredentialPool::new();
        let binding = account(&pool, secret).await;
        let quota = binding.quota.clone();
        let provider = built(family, &pool, Some(binding));
        let signed_in = AuthState::Authenticated {
            account_label: Some("user@example.com".into()),
        };
        assert_eq!(provider.auth_status().await, signed_in, "{family}");
        assert_eq!(
            provider.quota().unwrap().latest(),
            quota.latest(),
            "{family}"
        );
    }
}

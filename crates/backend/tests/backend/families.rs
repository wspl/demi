//! Scripted provider families: an API-key family whose directory the test
//! answers, and a subscription family whose device login, token import and
//! quota probe the test scripts, both over the account operations and the
//! quota every family shares. No test calls a real vendor.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use demi_backend::{FamilyArgs, FamilyCredential, FamilyError, ProviderFamily};
use demi_core::{
    AuthState, LoginPending, ProviderErrorDiagnostics, ProviderFailureFacts, ProviderModel, ProviderModelList,
    QuotaWindow, RuntimeState, Timestamp, WireApi,
};
use demi_provider::credentials::{
    AccountKit, AccountLabel, Accounts, AccountsCapability, AccountsError, AddAccount, LoginError, NewAccount,
    SubscriptionAccounts,
};
use demi_provider::quota::{Observation, ProbeCost, ProbeReading, ProviderQuota, QuotaError, QuotaSource};
use demi_provider::testing::{ScriptedRuntime, Turn, event};
use demi_provider::{Capabilities, CatalogError, Provider, ProviderRuntime, RuntimeEnv, RuntimeError};
use demi_web_api::providers::CredentialKind;
use futures_util::future::BoxFuture;
use tokio::sync::watch;

/// A catalog of one model per name.
pub fn catalog(names: &[&str]) -> ProviderModelList {
    ProviderModelList {
        models: names
            .iter()
            .map(|name| ProviderModel {
                id: (*name).into(),
                display_name: format!("{name} model"),
                description: None,
                context_window: Some(100_000),
                output_limit: Some(8_000),
                supports_tools: Some(true),
                supports_attachments: Some(true),
                supports_video: None,
                accepted_extensions: None,
                supports_reasoning: Some(true),
                supported_thinking_efforts: Some(vec!["low".into(), "high".into()]),
                default_thinking_effort: Some("low".into()),
                can_disable_thinking: None,
                service_tiers: Vec::new(),
                default_service_tier_id: None,
                cost: None,
            })
            .collect(),
        default_model_id: None,
        warnings: Vec::new(),
        source_fetched_at: "2026-09-24T07:00:00.000Z".parse().unwrap(),
        stale: false,
    }
}

/// A provider directory that answers each read with the next scripted
/// answer, and counts its reads.
#[derive(Default)]
pub struct Directory {
    answers: Mutex<VecDeque<Result<ProviderModelList, CatalogError>>>,
    reads: AtomicUsize,
}

impl Directory {
    pub fn answer(&self, answer: Result<ProviderModelList, CatalogError>) {
        self.answers.lock().unwrap().push_back(answer);
    }

    pub fn reads(&self) -> usize {
        self.reads.load(Ordering::SeqCst)
    }

    fn read(&self) -> Result<ProviderModelList, CatalogError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        let answer = self.answers.lock().unwrap().pop_front();
        answer.unwrap_or_else(|| Err(CatalogError::Unavailable("the directory has no answer scripted".into())))
    }
}

/// An API-key family whose provider answers every run with `ok`.
pub struct ScriptedKey {
    pub directory: Arc<Directory>,
    /// The wires an entry may name, as the `openai` family's two.
    pub wires: &'static [WireApi],
}

impl ProviderFamily for ScriptedKey {
    fn credential(&self) -> CredentialKind {
        CredentialKind::ApiKey
    }

    fn wires(&self) -> &'static [WireApi] {
        self.wires
    }

    fn provider(&self, args: FamilyArgs) -> Result<Arc<dyn Provider>, FamilyError> {
        let FamilyCredential::ApiKey(_) = args.credential else {
            return Err(FamilyError::WrongCredential);
        };
        Ok(Arc::new(Scripted {
            id: args.entry_id,
            label: args.label,
            directory: self.directory.clone(),
            accounts: None,
            quota: None,
            account: None,
        }))
    }
}

/// What a scripted subscription family's device login does: it reports
/// its code, then waits until the test approves it, counting logins that
/// were cancelled first.
pub struct LoginScript {
    approved: watch::Sender<bool>,
    pub cancelled: AtomicUsize,
}

impl Default for LoginScript {
    fn default() -> Self {
        Self {
            approved: watch::Sender::new(false),
            cancelled: AtomicUsize::new(0),
        }
    }
}

impl LoginScript {
    pub fn approve(&self, approved: bool) {
        self.approved.send_replace(approved);
    }
}

/// What a scripted family's quota probe answers.
pub struct QuotaScript {
    pub cost: Option<ProbeCost>,
    pub reading: Mutex<Result<ProbeReading, QuotaError>>,
}

impl QuotaScript {
    pub fn free(windows: Vec<QuotaWindow>) -> Self {
        Self {
            cost: Some(ProbeCost::Free),
            reading: Mutex::new(Ok(ProbeReading {
                plan: None,
                account_label: Some("device@example.test".into()),
                windows,
            })),
        }
    }
}

/// A subscription family over the entry's pool: device login and setup
/// tokens both add accounts, and the account's quota comes from the script.
pub struct ScriptedSubscription {
    pub login: Arc<LoginScript>,
    pub quota: Arc<QuotaScript>,
    pub directory: Arc<Directory>,
}

impl ProviderFamily for ScriptedSubscription {
    fn credential(&self) -> CredentialKind {
        CredentialKind::Subscription
    }

    fn provider(&self, args: FamilyArgs) -> Result<Arc<dyn Provider>, FamilyError> {
        let FamilyCredential::Subscription(subscription) = args.credential else {
            return Err(FamilyError::WrongCredential);
        };
        let kit = Kit {
            script: self.login.clone(),
        };
        let accounts = Accounts::new(subscription.pool, kit, args.clock.clone());
        let quota = subscription.account.as_ref().map(|binding| {
            ProviderQuota::new(
                Box::new(Probe(self.quota.clone())),
                binding.quota.clone(),
                args.clock.clone(),
            )
        });
        Ok(Arc::new(Scripted {
            id: args.entry_id,
            label: args.label,
            directory: self.directory.clone(),
            accounts: Some(Box::new(accounts)),
            quota,
            account: subscription.account.map(|binding| binding.credential_id),
        }))
    }
}

struct Kit {
    script: Arc<LoginScript>,
}

/// Counts a login that ends before it was approved.
struct Cancelled(Option<Arc<LoginScript>>);

impl Drop for Cancelled {
    fn drop(&mut self) {
        if let Some(script) = self.0.take() {
            script.cancelled.fetch_add(1, Ordering::SeqCst);
        }
    }
}

impl AccountKit for Kit {
    fn capability(&self) -> AccountsCapability {
        AccountsCapability { login: true, add: true }
    }

    fn login<'a>(
        &'a self,
        pending: &'a (dyn Fn(LoginPending) + Send + Sync),
    ) -> Option<BoxFuture<'a, Result<NewAccount, LoginError>>> {
        let script = self.script.clone();
        Some(Box::pin(async move {
            pending(LoginPending {
                verification_url: "https://verify.example/device".into(),
                user_code: Some("ABCD-1234".into()),
                expires_at: None,
            });
            let mut guard = Cancelled(Some(script.clone()));
            let mut approved = script.approved.subscribe();
            let _ = approved.wait_for(|approved| *approved).await;
            guard.0 = None;
            Ok(NewAccount {
                secret: r#"{"token":"login-secret"}"#.into(),
                label: AccountLabel {
                    label: "device@example.test".into(),
                    detail: None,
                    identity_key: Some("device".into()),
                },
            })
        }))
    }

    fn add(&self, input: AddAccount) -> Option<Result<NewAccount, AccountsError>> {
        let AddAccount::SetupToken(token) = input;
        let token = token.expose();
        if token.starts_with("bad") {
            // A family's own message may quote what it was given.
            return Some(Err(AccountsError::Invalid(format!("{token} is not a setup token"))));
        }
        Some(Ok(NewAccount {
            secret: serde_json::json!({ "setupToken": token }).to_string(),
            label: AccountLabel {
                label: format!("Account {}", &token[token.len() - 1..]),
                detail: None,
                identity_key: Some(token.to_owned()),
            },
        }))
    }
}

struct Probe(Arc<QuotaScript>);

impl QuotaSource for Probe {
    fn probe_cost(&self) -> Option<ProbeCost> {
        self.0.cost
    }

    fn probe(&self) -> BoxFuture<'_, Result<ProbeReading, QuotaError>> {
        let reading = self.0.reading.lock().unwrap().clone();
        Box::pin(async move { reading })
    }

    fn observe(&self, _: Observation<'_>) -> Option<Vec<QuotaWindow>> {
        None
    }
}

/// A scripted family's provider.
struct Scripted {
    id: String,
    label: String,
    directory: Arc<Directory>,
    accounts: Option<Box<dyn SubscriptionAccounts>>,
    quota: Option<ProviderQuota>,
    account: Option<String>,
}

impl Provider for Scripted {
    fn id(&self) -> &str {
        &self.id
    }

    fn display_name(&self) -> &str {
        &self.label
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }

    fn auth_status(&self) -> BoxFuture<'_, AuthState> {
        let account_label = self.account.clone();
        Box::pin(async move { AuthState::Authenticated { account_label } })
    }

    fn runtime_state(&self) -> RuntimeState {
        RuntimeState::Ready { message: None }
    }

    fn list_models(&self) -> BoxFuture<'_, Result<ProviderModelList, CatalogError>> {
        let answer = self.directory.read();
        Box::pin(async move { answer })
    }

    fn read_failure(&self, _: &ProviderErrorDiagnostics, _: Timestamp) -> ProviderFailureFacts {
        ProviderFailureFacts { retry_at: None }
    }

    fn quota(&self) -> Option<&ProviderQuota> {
        self.quota.as_ref()
    }

    fn accounts(&self) -> Option<&dyn SubscriptionAccounts> {
        self.accounts.as_deref()
    }

    fn runtime(&self, _: RuntimeEnv) -> Result<Box<dyn ProviderRuntime>, RuntimeError> {
        Ok(Box::new(ScriptedRuntime::new([Turn::Events(vec![
            event::text("ok"),
            event::response(1, 1),
        ])])))
    }
}

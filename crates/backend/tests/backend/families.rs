//! Scripted provider families: an API-key family whose directory the test
//! answers, and a subscription family whose device login and quota probe the
//! test scripts, both over the account operations and the
//! quota every family shares, and makes images and videos as the test
//! scripts. No test calls a real vendor.

use demi_provider_common::media_catalog::Catalog;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use demi_backend_providers::llm::families::{
    FamilyArgs, FamilyCredential, FamilyError, ProviderFamily,
};
use demi_provider_claude_code::Placement;
use demi_provider_common::credentials::{
    AccountKit, AccountLabel, Accounts, AccountsCapability, LoginError, LoginIo, LoginKind,
    NewAccount, SubscriptionAccounts,
};
use demi_provider_common::quota::{
    Observation, ProbeCost, ProbeReading, ProviderQuota, QuotaError, QuotaSource,
};
use demi_provider_common::testing::{ScriptedRuntime, Turn, event};
use demi_provider_common::images::{ImageMaker, ImageRequest, MadeImage};
use demi_provider_common::media::{MediaAnswer, MediaFailure, MediaMaker, MediaModelInfo, MediaVendor};
use demi_provider_common::videos::{Done, JobState, VideoMaker, VideoRequest};
use demi_provider_common::{
    Capabilities, CatalogError, Provider, ProviderRuntime, RuntimeEnv, RuntimeError,
};
use demi_shared_types::{ImageShape, VideoShape,
    AuthState, LoginPending, ProviderErrorDiagnostics, ProviderFailureFacts, ProviderModel,
    ProviderModelList, QuotaWindow, RuntimeState, Timestamp, TokenUsage, WireApi,
};
use demi_web_api_protocol::providers::CredentialKind;
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
                accepted_extensions: None,
                supports_reasoning: Some(true),
                supported_thinking_efforts: Some(vec!["low".into(), "high".into()]),
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
        answer.unwrap_or_else(|| {
            Err(CatalogError::Unavailable(
                "the directory has no answer scripted".into(),
            ))
        })
    }
}

/// An API-key family whose provider answers every run with `ok`.
pub struct ScriptedKey {
    pub directory: Arc<Directory>,
    /// The wires an entry may name, as the `openai` family's two.
    pub wires: &'static [WireApi],
    /// Whether its provider says it runs a process on a Host, which the
    /// user's Cloud then is.
    pub process_host: bool,
    /// The images its key makes, as the account [`KEY`] of the script.
    pub images: Option<Arc<ImageScript>>,
}

/// The account a key's image calls are scripted under.
pub const KEY: &str = "key";

impl ProviderFamily for ScriptedKey {
    fn credential(&self) -> CredentialKind {
        CredentialKind::ApiKey
    }

    /// A vendor entry of a family whose key makes images makes them in the
    /// OpenAI shape, as an OpenAI key does.
    fn image_shape(&self, vendor_id: Option<&str>) -> Option<ImageShape> {
        self.images.as_ref().filter(|_| vendor_id.is_some()).map(|_| ImageShape::OpenaiImages)
    }

    fn wires(&self) -> &'static [WireApi] {
        self.wires
    }

    fn provider(&self, args: FamilyArgs) -> Result<Arc<dyn Provider>, FamilyError> {
        let FamilyCredential::ApiKey(_) = args.credential else {
            return Err(FamilyError::WrongCredential);
        };
        Ok(Arc::new(Scripted {
            directory: self.directory.clone(),
            accounts: None,
            quota: None,
            account: None,
            process_host: self.process_host,
            runs: None,
            images: self.images.clone().map(|script| ScriptedImages {
                script,
                account: KEY.to_owned(),
            }),
            videos: None,
        }))
    }

    /// The runtime of a provider that runs a process, which is scripted
    /// too: it answers as every runtime of the family does, wherever the
    /// placement would start the process.
    fn process_runtime(
        &self,
        _: FamilyArgs,
        _: RuntimeEnv,
        _: Rc<dyn Placement>,
    ) -> Option<Result<Box<dyn ProviderRuntime>, FamilyError>> {
        self.process_host.then(|| Ok(answers_ok()))
    }
}

/// What a scripted subscription family's device login does: it reports
/// its code, then waits until the test approves it, counting logins that
/// were cancelled first. An approved login signs in the account the script
/// names, `device` unless the test names another.
pub struct LoginScript {
    approved: watch::Sender<bool>,
    pub cancelled: AtomicUsize,
    account: Mutex<String>,
}

impl Default for LoginScript {
    fn default() -> Self {
        Self {
            approved: watch::Sender::new(false),
            cancelled: AtomicUsize::new(0),
            account: Mutex::new("device".into()),
        }
    }
}

impl LoginScript {
    pub fn approve(&self, approved: bool) {
        self.approved.send_replace(approved);
    }

    /// The account the next approved login signs in.
    pub fn signs_in(&self, account: &str) {
        *self.account.lock().unwrap() = account.into();
    }
}

/// What a scripted family's quota probe answers: the reading, with the
/// windows scripted for the probed account in place of its own, and how
/// many probes ran.
pub struct QuotaScript {
    pub cost: Option<ProbeCost>,
    pub reading: Mutex<Result<ProbeReading, QuotaError>>,
    pub windows_of: Mutex<std::collections::HashMap<String, Vec<QuotaWindow>>>,
    pub probes: AtomicUsize,
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
            windows_of: Mutex::default(),
            probes: AtomicUsize::new(0),
        }
    }
}

/// A subscription family over the entry's pool: a device login adds
/// accounts, and the account's quota comes from the script.
pub struct ScriptedSubscription {
    pub login: Arc<LoginScript>,
    pub quota: Arc<QuotaScript>,
    pub directory: Arc<Directory>,
    /// Where its runtimes answer with the account's ID and wait as the test
    /// says; without it, each run answers `ok`.
    pub runs: Option<Arc<crate::conversations::Runs>>,
    /// Whether its provider says it runs a process, as Claude Code's does,
    /// so a session gets its runtime for its account through the family's
    /// process runtime.
    pub process_host: bool,
    /// The image role of its provider; none for a family that makes no
    /// images.
    pub images: Option<Arc<ImageScript>>,
    /// Its video role, in the Grok shape; none for a family that makes no
    /// videos.
    pub videos: Option<Arc<VideoScript>>,
}

/// A scripted image role: who answers and whose models it makes, its
/// models, the image every call answers, the accounts whose calls fail as
/// used up, and the calls it was sent.
pub struct ImageScript {
    pub name: &'static str,
    pub vendor: MediaVendor,
    /// The IDs of its models, which it lists as a key's listing names
    /// them, told as the backend's catalog states them.
    pub models: &'static [&'static str],
    pub image: Mutex<bytes::Bytes>,
    pub used_up: Mutex<Vec<String>>,
    /// Whether a call waits, never answering, until it is dropped.
    pub hold: std::sync::atomic::AtomicBool,
    /// Holds the next call until the test sends on it, then answers as the
    /// script says then.
    pub gate: Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
    /// The vendor's words of a refusal every call answers, when set.
    pub refusal: Mutex<Option<String>>,
    calls: Mutex<Vec<(String, ImageRequest)>>,
    /// How many calls were asked, and how many held calls were dropped.
    pub counts: watch::Sender<(usize, usize)>,
}

/// Counts a held call as dropped when it is.
struct Dropped(Arc<ImageScript>);

impl Drop for Dropped {
    fn drop(&mut self) {
        self.0.counts.send_modify(|(_, dropped)| *dropped += 1);
    }
}

impl ImageScript {
    /// OpenAI's images, answered as `Scripted`.
    pub fn new(models: &'static [&'static str], image: bytes::Bytes) -> Self {
        Self {
            name: "Scripted",
            vendor: MediaVendor::OpenAi,
            models,
            image: Mutex::new(image),
            used_up: Mutex::default(),
            hold: std::sync::atomic::AtomicBool::new(false),
            gate: Mutex::default(),
            refusal: Mutex::default(),
            calls: Mutex::default(),
            counts: watch::Sender::new((0, 0)),
        }
    }

    /// The script answering as `name`.
    pub fn named(mut self, name: &'static str) -> Self {
        self.name = name;
        self
    }

    /// Each call's account, by its ID, and request, in order.
    pub fn calls(&self) -> Vec<(String, ImageRequest)> {
        self.calls.lock().unwrap().clone()
    }
}

/// The image role of one account's provider.
struct ScriptedImages {
    script: Arc<ImageScript>,
    account: String,
}

impl MediaMaker for ScriptedImages {
    fn name(&self) -> &str {
        self.script.name
    }

    fn vendor(&self) -> Option<MediaVendor> {
        Some(self.script.vendor)
    }

    fn models<'a>(
        &'a self,
        _: &'a demi_provider_common::ProviderHttp,
        _: &'a Catalog,
    ) -> BoxFuture<'a, Result<Vec<MediaModelInfo>, MediaFailure>> {
        let models = self.script.models.iter().map(|id| MediaModelInfo::id(id)).collect();
        Box::pin(std::future::ready(Ok(models)))
    }
}

impl ImageMaker for ScriptedImages {
    fn make<'a>(
        &'a self,
        _: &'a demi_provider_common::ProviderHttp,
        _: &'a Catalog,
        request: ImageRequest,
    ) -> BoxFuture<'a, MediaAnswer<MadeImage>> {
        let mut calls = self.script.calls.lock().unwrap();
        calls.push((self.account.clone(), request));
        self.script.counts.send_modify(|(asked, _)| *asked += 1);
        if self.script.hold.load(Ordering::SeqCst) {
            let dropped = Dropped(self.script.clone());
            return Box::pin(async move {
                let _dropped = dropped;
                std::future::pending().await
            });
        }
        let count = calls.len();
        drop(calls);
        let gate = self.script.gate.lock().unwrap().take();
        Box::pin(async move {
            if let Some(gate) = gate {
                // A dropped sender releases the call as well.
                let _ = gate.await;
            }
            let script = &self.script;
            let result = if let Some(message) = script.refusal.lock().unwrap().clone() {
                Err(MediaFailure::Refused { message, code: None })
            } else if script.used_up.lock().unwrap().contains(&self.account) {
                Err(MediaFailure::UsageLimit { retry_at: None })
            } else {
                Ok(MadeImage {
                    bytes: script.image.lock().unwrap().clone(),
                    usage: Some(TokenUsage {
                        input_tokens: 25,
                        output_tokens: 515,
                        cache_read_tokens: 0,
                        cache_write_tokens: 0,
                    }),
                    vendor_id: Some(format!("gen-{count}")),
                })
            };
            MediaAnswer { result, observed: Vec::new() }
        })
    }
}

/// A scripted video role, xAI's in the Grok shape: its models, the answers
/// of its jobs' state reads in turn, the video every download answers, the
/// accounts whose asks fail as used up, how long an ask takes, a failure
/// the next download answers, and the asks it was sent.
pub struct VideoScript {
    /// The IDs of its models, told as the backend's catalog states them.
    pub models: &'static [&'static str],
    /// What each read of a job's state answers, one per read; with none
    /// left, the job runs on.
    pub states: Mutex<VecDeque<Result<JobState, MediaFailure>>>,
    pub video: bytes::Bytes,
    pub used_up: Mutex<Vec<String>>,
    pub ask_takes: Mutex<std::time::Duration>,
    pub download_fails: Mutex<Option<MediaFailure>>,
    asks: Mutex<Vec<(String, VideoRequest)>>,
    /// How many asks and how many state reads were sent.
    pub counts: watch::Sender<(usize, usize)>,
}

impl VideoScript {
    pub fn new(models: &'static [&'static str], video: bytes::Bytes) -> Self {
        Self {
            models,
            states: Mutex::default(),
            video,
            used_up: Mutex::default(),
            ask_takes: Mutex::default(),
            download_fails: Mutex::default(),
            asks: Mutex::default(),
            counts: watch::Sender::new((0, 0)),
        }
    }

    /// The reads' answers to come, in order.
    pub fn answer(&self, states: impl IntoIterator<Item = Result<JobState, MediaFailure>>) {
        self.states.lock().unwrap().extend(states);
    }

    /// Each ask's account, by its ID, and request, in order.
    pub fn asks(&self) -> Vec<(String, VideoRequest)> {
        self.asks.lock().unwrap().clone()
    }

    /// A finished job's answer.
    pub fn done() -> JobState {
        JobState::Done(Done {
            url: "https://vendor.invalid/video.mp4".parse().unwrap(),
            cost: Some(demi_shared_types::Cost {
                amount: 0.05,
                currency: demi_shared_types::Currency::Usd,
            }),
            usage: None,
        })
    }
}

/// The video role of one account's provider.
struct ScriptedVideos {
    script: Arc<VideoScript>,
    account: String,
}

impl MediaMaker for ScriptedVideos {
    fn name(&self) -> &str {
        "Grok"
    }

    fn vendor(&self) -> Option<MediaVendor> {
        Some(MediaVendor::Xai)
    }

    fn models<'a>(
        &'a self,
        _: &'a demi_provider_common::ProviderHttp,
        _: &'a Catalog,
    ) -> BoxFuture<'a, Result<Vec<MediaModelInfo>, MediaFailure>> {
        let models = self.script.models.iter().map(|id| MediaModelInfo::id(id)).collect();
        Box::pin(std::future::ready(Ok(models)))
    }
}

impl VideoMaker for ScriptedVideos {
    fn shape(&self) -> VideoShape {
        VideoShape::Grok
    }

    fn ask<'a>(&'a self, _: &'a demi_provider_common::ProviderHttp, request: VideoRequest) -> BoxFuture<'a, MediaAnswer<String>> {
        let mut asks = self.script.asks.lock().unwrap();
        asks.push((self.account.clone(), request));
        let count = asks.len();
        drop(asks);
        self.script.counts.send_modify(|(asked, _)| *asked += 1);
        let result = if self.script.used_up.lock().unwrap().contains(&self.account) {
            Err(MediaFailure::UsageLimit { retry_at: None })
        } else {
            Ok(format!("job-{count}"))
        };
        let takes = *self.script.ask_takes.lock().unwrap();
        Box::pin(async move {
            tokio::time::sleep(takes).await;
            MediaAnswer { result, observed: Vec::new() }
        })
    }

    fn state<'a>(&'a self, _: &'a demi_provider_common::ProviderHttp, _: &'a str) -> BoxFuture<'a, MediaAnswer<JobState>> {
        let next = self.script.states.lock().unwrap().pop_front();
        self.script.counts.send_modify(|(_, read)| *read += 1);
        let result = next.unwrap_or(Ok(JobState::Running { progress: None }));
        Box::pin(std::future::ready(MediaAnswer { result, observed: Vec::new() }))
    }

    fn download<'a>(&'a self, _: &'a demi_provider_common::ProviderHttp, _: &'a Done) -> BoxFuture<'a, MediaAnswer<bytes::Bytes>> {
        let result = match self.script.download_fails.lock().unwrap().take() {
            Some(failure) => Err(failure),
            None => Ok(self.script.video.clone()),
        };
        Box::pin(std::future::ready(MediaAnswer { result, observed: Vec::new() }))
    }
}

impl ProviderFamily for ScriptedSubscription {
    fn credential(&self) -> CredentialKind {
        CredentialKind::Subscription
    }

    fn image_shape(&self, _: Option<&str>) -> Option<ImageShape> {
        self.images.as_ref().map(|_| ImageShape::Chatgpt)
    }

    fn video_shape(&self, _: Option<&str>) -> Option<VideoShape> {
        self.videos.as_ref().map(|_| VideoShape::Grok)
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
                Box::new(Probe(self.quota.clone(), binding.credential_id.clone())),
                binding.quota.clone(),
                args.clock.clone(),
            )
        });
        Ok(Arc::new(Scripted {
            directory: self.directory.clone(),
            accounts: Some(Box::new(accounts)),
            quota,
            images: self.images.clone().zip(subscription.account.as_ref()).map(|(script, binding)| ScriptedImages {
                script,
                account: binding.credential_id.clone(),
            }),
            videos: self.videos.clone().zip(subscription.account.as_ref()).map(|(script, binding)| ScriptedVideos {
                script,
                account: binding.credential_id.clone(),
            }),
            account: subscription.account.map(|binding| binding.credential_id),
            process_host: self.process_host,
            runs: self.runs.clone(),
        }))
    }

    /// A runtime keyed by the account it is built for, as a Claude Code
    /// process holds its one account's token.
    fn process_runtime(
        &self,
        args: FamilyArgs,
        _: RuntimeEnv,
        _: Rc<dyn Placement>,
    ) -> Option<Result<Box<dyn ProviderRuntime>, FamilyError>> {
        if !self.process_host {
            return None;
        }
        let FamilyCredential::Subscription(subscription) = args.credential else {
            return Some(Err(FamilyError::WrongCredential));
        };
        let account = subscription.account.map(|binding| binding.credential_id);
        Some(Ok(match (&self.runs, account) {
            (Some(runs), Some(account)) => {
                Box::new(crate::conversations::KeyedRuntime::new(account, runs.clone()))
            }
            _ => answers_ok(),
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
        AccountsCapability {
            login: Some(LoginKind::Device),
        }
    }

    fn login<'a>(
        &'a self,
        io: LoginIo<'a>,
    ) -> Option<BoxFuture<'a, Result<NewAccount, LoginError>>> {
        let script = self.script.clone();
        Some(Box::pin(async move {
            let login = async {
                (io.pending)(LoginPending {
                    verification_url: "https://verify.example/device".into(),
                    user_code: Some("ABCD-1234".into()),
                    code_error: None,
                });
                let mut guard = Cancelled(Some(script.clone()));
                let mut approved = script.approved.subscribe();
                let _ = approved.wait_for(|approved| *approved).await;
                guard.0 = None;
                let account = script.account.lock().unwrap().clone();
                Ok(NewAccount {
                    secret: Some(
                        serde_json::json!({ "token": format!("login-secret-{account}") })
                            .to_string(),
                    ),
                    label: AccountLabel {
                        label: format!("{account}@example.test"),
                        detail: None,
                        identity_key: Some(account),
                    },
                })
            };
            LoginIo::until_stopped(&io.stop, login).await
        }))
    }
}

/// The probe of one account, by its ID.
struct Probe(Arc<QuotaScript>, String);

impl QuotaSource for Probe {
    fn probe_cost(&self) -> Option<ProbeCost> {
        self.0.cost
    }

    fn probe(&self) -> BoxFuture<'_, Result<ProbeReading, QuotaError>> {
        self.0.probes.fetch_add(1, Ordering::SeqCst);
        let mut reading = self.0.reading.lock().unwrap().clone();
        if let (Ok(reading), Some(windows)) = (&mut reading, self.0.windows_of.lock().unwrap().get(&self.1)) {
            reading.windows = windows.clone();
        }
        Box::pin(async move { reading })
    }

    fn observe(&self, _: Observation<'_>) -> Option<Vec<QuotaWindow>> {
        None
    }
}

/// A scripted family's provider.
struct Scripted {
    directory: Arc<Directory>,
    accounts: Option<Box<dyn SubscriptionAccounts>>,
    quota: Option<ProviderQuota>,
    account: Option<String>,
    process_host: bool,
    runs: Option<Arc<crate::conversations::Runs>>,
    images: Option<ScriptedImages>,
    videos: Option<ScriptedVideos>,
}

impl Provider for Scripted {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            process_host: self.process_host,
        }
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
        ProviderFailureFacts { retry_at: None, continue_with: None, signed_out: None, not_exposed: None, device_offline: None }
    }

    fn quota(&self) -> Option<&ProviderQuota> {
        self.quota.as_ref()
    }

    fn accounts(&self) -> Option<&dyn SubscriptionAccounts> {
        self.accounts.as_deref()
    }

    fn images(&self) -> Option<&dyn ImageMaker> {
        self.images.as_ref().map(|images| images as &dyn ImageMaker)
    }

    fn videos(&self) -> Option<&dyn VideoMaker> {
        self.videos.as_ref().map(|videos| videos as &dyn VideoMaker)
    }

    fn runtime(&self, _: RuntimeEnv) -> Result<Box<dyn ProviderRuntime>, RuntimeError> {
        Ok(match (&self.runs, &self.account) {
            (Some(runs), Some(account)) => Box::new(crate::conversations::KeyedRuntime::new(
                account.clone(),
                runs.clone(),
            )),
            _ => answers_ok(),
        })
    }
}

/// A runtime that answers its one run with `ok`.
fn answers_ok() -> Box<dyn ProviderRuntime> {
    Box::new(ScriptedRuntime::new([Turn::Events(vec![
        event::text("ok"),
        event::response(1, 1),
    ])]))
}

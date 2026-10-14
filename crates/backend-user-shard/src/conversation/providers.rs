//! The provider runtimes of a user's conversations (`providers.md`
//! § Inference admission and runtime ownership). A session's runtime
//! resolves the conversation's provider entry at every inference boundary: a
//! fresh read of the entry in the user's scope, the entry's configured model
//! facts applied again, and a runtime that serves the request. It keeps its
//! runtime while the entry and the model stay the same, and builds a new one,
//! closing the old, when either changes; a running request finishes with the
//! runtime it started with. Every runtime is metered: the user's rate limit
//! applies before each run, and each response is a usage ledger row before
//! the agent sees it.

use std::cell::RefCell;
use std::num::NonZeroU32;
use std::rc::{Rc, Weak};
use std::sync::Arc;

use demi_agent_server::{ProviderResolver, ResolveError};
use demi_agent_tools::{ProfileModel, Unavailable};
use demi_backend_host_access::transition::ChangeRefusal;
use demi_backend_providers::llm::catalog::configured_selection;
use demi_backend_providers::usage::meter::{Ledger, MeteredRuntime};
use demi_backend_providers::usage::rate_limit::RequestRateLimit;
use demi_backend_providers::vault::entries::{EntryCredential, ProviderEntry};
use demi_provider_common::{
    AccountSwitch, Ahead, ErrorCode, InferenceRequest, Provider, ProviderEvent, ProviderFailure, ProviderRun,
    ProviderRuntime, RequestLimits, RuntimeEnv,
};
use demi_backend_providers::llm::rotation::{Candidate, Choice, Holding, USED_UP_FOR_MS, used_up_window};
use demi_shared_types::{
    AgentAccount, Model, ModelSelection, NodeId, Rotation, Timestamp, UnavailableSetting,
    session_context_window,
};
use demi_web_api_protocol::conversations::ModelSettings;
use demi_web_api_protocol::ids::{ConversationId, CredentialId, ProviderId, UserId};
use futures_util::future::LocalBoxFuture;
use futures_util::StreamExt as _;

use crate::conversation::claude_cli::{CloudPlacement, ProcessWork};
use crate::services::Services;
use crate::shard::Shard;
use demi_backend_host_access::conversation_of;

/// Where the user's sessions get their runtimes.
pub(super) struct ConversationProviders {
    /// The user's shard, whose Cloud runs a provider's process.
    shard: Weak<Shard>,
    user: UserId,
    services: Arc<Services>,
    /// The shard's HTTP client, which its runtimes send with.
    http: reqwest::Client,
    rate_limit: Rc<RefCell<RequestRateLimit>>,
}

impl ConversationProviders {
    pub(super) fn new(
        shard: Weak<Shard>,
        user: UserId,
        services: Arc<Services>,
        http: reqwest::Client,
        rate_limit: Rc<RefCell<RequestRateLimit>>,
    ) -> Self {
        Self {
            shard,
            user,
            services,
            http,
            rate_limit,
        }
    }
}

impl ProviderResolver for ConversationProviders {
    /// The model selection the record of `root`'s conversation holds; a
    /// socket opens only a conversation that has one.
    fn selection<'a>(
        &'a self,
        root: &'a NodeId,
    ) -> LocalBoxFuture<'a, Result<ModelSelection, ResolveError>> {
        Box::pin(async move {
            let record = self
                .services
                .control
                .conversation(conversation_of(root))
                .await
                .map_err(|error| ResolveError::Failed(error.to_string()))?;
            record
                .and_then(|record| record.model)
                .ok_or_else(|| ResolveError::Failed("The conversation has no model yet".into()))
        })
    }

    /// A runtime for a session of `root`'s conversation that infers with
    /// `model`: an entry outside the user's scope is unknown, and one that
    /// cannot run fails now, before the session relies on it.
    fn runtime<'a>(
        &'a self,
        root: &'a NodeId,
        model: &'a ModelSelection,
    ) -> LocalBoxFuture<'a, Result<Box<dyn ProviderRuntime>, ResolveError>> {
        Box::pin(async move {
            let unknown = || ResolveError::Unknown(model.provider_id.clone());
            let provider =
                ProviderId::try_from(model.provider_id.as_str()).map_err(|_| unknown())?;
            let scope = Rc::new(Scope {
                shard: self.shard.clone(),
                services: self.services.clone(),
                user: self.user.clone(),
                conversation: conversation_of(root),
                provider,
                http: self.http.clone(),
                rate_limit: self.rate_limit.clone(),
            });
            let entry = scope
                .services
                .vault
                .visible(&scope.user, &scope.provider)
                .await
                .map_err(|error| ResolveError::Failed(error.to_string()))?
                .ok_or_else(unknown)?;
            let mut runtime = ConversationRuntime {
                scope,
                selection: model.clone(),
                current: None,
                prepares_ahead: false,
                holding: None,
                unannounced: false,
            };
            // The active account's runtime until the session's first
            // request names or takes its own.
            let selection = configured_selection(&entry, model.clone())
                .map_err(|error| ResolveError::Failed(error.to_string()))?;
            runtime
                .serve(&entry, selection, None)
                .await
                .map_err(|failure| ResolveError::Failed(failure.message))?;
            Ok(Box::new(runtime) as Box<dyn ProviderRuntime>)
        })
    }

    /// The model's window, or the profile's Context `own` of a subagent, or
    /// else the limit the user stored on the model, read from the user's
    /// preferences at each check; either while the window offers it.
    fn context_window<'a>(
        &'a self,
        model: &'a ModelSelection,
        own: Option<u32>,
    ) -> LocalBoxFuture<'a, u32> {
        Box::pin(async move {
            let window = model.model.context_window;
            if own.is_some() {
                return session_context_window(window, own, None);
            }
            let preferences = match self.services.control.preferences(self.user.clone()).await {
                Ok(preferences) => preferences,
                Err(error) => {
                    // The model's own window is what the session used before
                    // any limit, and a history too large for the vendor still
                    // compacts when its request is refused.
                    tracing::warn!(
                        user = %self.user,
                        %error,
                        "the context limits could not be read; the model's window is used"
                    );
                    return window;
                }
            };
            let limit = preferences
                .context_limits
                .iter()
                .find(|(provider, _)| provider.as_str() == model.provider_id)
                .and_then(|(_, models)| models.get(&model.model.id))
                .copied();
            session_context_window(window, None, limit)
        })
    }

    /// The family of `model`'s entry, read from the user's scope as a
    /// request's admission reads it, and refused as admission refuses it.
    fn family<'a>(
        &'a self,
        model: &'a ModelSelection,
    ) -> LocalBoxFuture<'a, Result<String, ResolveError>> {
        Box::pin(async move {
            let provider = ProviderId::try_from(model.provider_id.as_str())
                .map_err(|_| ResolveError::Unknown(model.provider_id.clone()))?;
            let read = self.services.vault.visible(&self.user, &provider).await;
            match read {
                Ok(Some(entry)) => Ok(entry.family),
                Ok(None) => Err(ResolveError::Failed(entry_gone(&provider))),
                Err(error) => Err(ResolveError::Failed(entry_unreadable(&error))),
            }
        })
    }

    /// The selection a profile's model settings make from the entry's
    /// catalog now, as a model switch makes one; what is missing when a part
    /// is gone.
    fn profile_selection<'a>(
        &'a self,
        model: &'a ProfileModel,
    ) -> LocalBoxFuture<'a, Result<ModelSelection, Unavailable>> {
        Box::pin(async move {
            let shard = self
                .shard
                .upgrade()
                .ok_or_else(|| Unavailable::Failed("the backend is shutting down".into()))?;
            let provider_id =
                ProviderId::try_from(model.provider_id.as_str()).map_err(|_| Unavailable::Entry)?;
            let chosen = ModelSettings {
                provider_id,
                model_id: model.model_id.clone(),
                thinking_effort: model.thinking_effort.clone(),
                service_tier_id: model.service_tier_id.clone(),
            };
            shard
                .chosen_selection(&chosen)
                .await
                .map_err(|refusal| missing(&model.model_id, refusal))
        })
    }
}

/// What a profile's model settings miss, from why their selection was
/// refused.
fn missing(model: &str, refusal: ChangeRefusal) -> Unavailable {
    match refusal {
        ChangeRefusal::ProviderNotFound => Unavailable::Entry,
        ChangeRefusal::ModelNotFound => Unavailable::Model(model.to_owned()),
        ChangeRefusal::SettingUnavailable(UnavailableSetting::Effort(effort)) => {
            Unavailable::Effort {
                model: model.to_owned(),
                effort,
            }
        }
        ChangeRefusal::SettingUnavailable(UnavailableSetting::Tier(tier)) => Unavailable::Tier {
            model: model.to_owned(),
            tier,
        },
        other => Unavailable::Failed(other.to_string()),
    }
}

/// Whose requests a session's runtime makes, and what it builds its
/// runtimes with.
struct Scope {
    shard: Weak<Shard>,
    services: Arc<Services>,
    user: UserId,
    conversation: ConversationId,
    provider: ProviderId,
    http: reqwest::Client,
    rate_limit: Rc<RefCell<RequestRateLimit>>,
}

/// A session's runtime.
struct ConversationRuntime {
    scope: Rc<Scope>,
    /// The selection of the latest request, with the entry's configured
    /// facts applied.
    selection: ModelSelection,
    current: Option<Current>,
    /// Whether the entry's provider runs a process it prepares ahead; its
    /// family, and so this, never changes.
    prepares_ahead: bool,
    /// The agent's hold on the account it took under the entry's rotation;
    /// none under Manual (`providers.md` § Account rotation).
    holding: Option<Holding>,
    /// Whether the held account was taken ahead of a request, so the next
    /// request that names none tells the session of it.
    unannounced: bool,
}

/// The runtime that serves the session's requests, with what it was built
/// for.
struct Current {
    provider: Arc<dyn Provider>,
    model: String,
    runtime: Box<dyn ProviderRuntime>,
}

/// What admitting a request found.
struct Admitted {
    output_limit: Option<NonZeroU32>,
    /// Whether the entry gives each agent an account of its own.
    rotates: bool,
    /// The account to tell the session of before the run, when it is not
    /// the one the request named.
    announce: Option<Option<AgentAccount>>,
}

/// Why a request of a conversation whose provider entry left the user's
/// scope is refused.
fn entry_gone(provider: &ProviderId) -> String {
    format!("Provider \"{provider}\" is no longer available to this conversation")
}

/// Why a request is refused when its provider entry could not be read.
fn entry_unreadable(error: &impl std::fmt::Display) -> String {
    format!("The provider entry could not be read: {error}")
}

/// A failure of the inference boundary itself, before any vendor: it has no
/// code, so the agent never retries it by itself.
fn refused(message: impl Into<String>, code: Option<ErrorCode>) -> ProviderFailure {
    ProviderFailure {
        message: message.into(),
        code,
        diagnostics: None,
        retry_after: None,
    }
}

impl ConversationRuntime {
    /// Resolves the entry for `request` (`providers.md` § Inference admission
    /// and runtime ownership): the entry must still be in the user's scope,
    /// and a subscription entry must have an account. The account is the
    /// active one, or under the entry's rotation the agent's own: the one
    /// the request names while it is still the entry's and taken under the
    /// entry's rotation, else one taken now. Answers the model's output
    /// limit, which the entry's configured model decides when it has a
    /// configured list; the request's own cap is not the model's, and stays
    /// (`models.md` § Output limit).
    async fn admit(&mut self, request: &InferenceRequest) -> Result<Admitted, ProviderFailure> {
        let entry = self.entry().await?;
        let requested = ModelSelection {
            provider_id: self.selection.provider_id.clone(),
            model: Model {
                id: request.model_id.clone(),
                output_limit: request.output_limit.map(NonZeroU32::get),
                ..self.selection.model.clone()
            },
            thinking: request.thinking.clone(),
            service_tier_id: request.service_tier_id.clone(),
        };
        let selection = configured_selection(&entry, requested)
            .map_err(|error| refused(error.to_string(), None))?;
        let rotation = entry.rotation();
        let (account, announce) = match rotation {
            Rotation::Manual => {
                self.holding = None;
                let announce = request.account.is_some().then_some(None);
                (entry.active().cloned(), announce)
            }
            Rotation::InOrder | Rotation::Balanced => {
                let (account, announce) = self
                    .agent_account(&entry, &selection.model, request.account.as_ref())
                    .await?;
                (Some(account), announce)
            }
        };
        self.serve(&entry, selection, account.as_ref()).await?;
        Ok(Admitted {
            output_limit: self.selection.model.output_limit.and_then(NonZeroU32::new),
            rotates: rotation != Rotation::Manual,
            announce,
        })
    }

    /// A fresh read of the entry in the user's scope; a runtime whose entry
    /// is gone has nothing left to serve.
    async fn entry(&mut self) -> Result<ProviderEntry, ProviderFailure> {
        let scope = self.scope.clone();
        let read = scope
            .services
            .vault
            .visible(&scope.user, &scope.provider)
            .await;
        match read {
            Ok(Some(entry)) => Ok(entry),
            Ok(None) => {
                self.close_current().await;
                Err(refused(entry_gone(&scope.provider), None))
            }
            Err(error) => Err(refused(entry_unreadable(&error), None)),
        }
    }

    /// The account the agent uses under `entry`'s rotation: `named`, the
    /// one the session keeps, while the entry still holds it and its
    /// rotation is the one it was taken under; else one taken now, which
    /// the session is told of.
    async fn agent_account(
        &mut self,
        entry: &ProviderEntry,
        model: &Model,
        named: Option<&AgentAccount>,
    ) -> Result<(CredentialId, Option<Option<AgentAccount>>), ProviderFailure> {
        let assembly = &self.scope.services.assembly;
        let rotation = entry.rotation();
        if let Some(named) = named.filter(|named| named.rotation == rotation) {
            let accounts = assembly
                .vault()
                .accounts(entry.id.clone())
                .await
                .map_err(|error| refused(entry_unreadable(&error), None))?;
            if let Some(kept) = accounts
                .into_iter()
                .find(|account| account.id.as_str() == named.credential_id)
            {
                let held = self
                    .holding
                    .as_ref()
                    .is_some_and(|holding| *holding.account() == kept.id);
                if !held {
                    self.holding = Some(assembly.rotation().hold(&entry.id, &kept.id));
                }
                self.unannounced = false;
                return Ok((kept.id, None));
            }
        }
        if named.is_none()
            && self.unannounced
            && let Some(holding) = &self.holding
        {
            // Taken ahead of this request: the session learns of it now.
            self.unannounced = false;
            let account = holding.account().clone();
            return Ok((account.clone(), Some(Some(agent_account(&account, rotation)))));
        }
        let (choice, candidates) = assembly
            .take_account(entry, model)
            .await
            .map_err(|error| refused(error.to_string(), None))?;
        match choice {
            Some(Choice::Account(holding)) => {
                let account = holding.account().clone();
                self.holding = Some(holding);
                self.unannounced = true;
                Ok((account.clone(), Some(Some(agent_account(&account, rotation)))))
            }
            Some(Choice::AllUsedUp { .. }) => Err(all_used_up(entry, &candidates)),
            None => unreachable!("a rotating entry takes an account or finds them used up"),
        }
    }

    /// The agent's account failed with `failure`, a usage limit: it is
    /// used up until the moment the failure or its quota names, or for 30
    /// minutes, and the agent takes another account, whose runtime serves
    /// the same request again. Answers the move, or the failure when every
    /// account is used up (`providers.md` § Account rotation).
    async fn switch_account(
        &mut self,
        failure: &ProviderFailure,
    ) -> Result<(AccountSwitch, AgentAccount), ()> {
        let Some(used_up) = self.holding.as_ref().map(|holding| holding.account().clone()) else {
            return Err(());
        };
        let entry = self.entry().await.map_err(drop)?;
        let assembly = self.scope.services.assembly.clone();
        let now = self.scope.services.clock.now();
        let model = self.selection.model.clone();
        let candidates = assembly.candidates(&entry, false).await.map_err(drop)?;
        let failed = candidates.iter().find(|candidate| candidate.id == used_up);
        let named = match (&failure.diagnostics, &self.current) {
            (Some(diagnostics), Some(current)) => {
                current.provider.read_failure(diagnostics, now).retry_at
            }
            _ => None,
        };
        let quota = used_up_window(failed.and_then(|failed| failed.snapshot.as_ref()), &model, now);
        let moment = named.or(quota.as_ref().and_then(|(_, reset)| *reset));
        let until = moment.unwrap_or_else(|| {
            Timestamp::from_millisecond(now.as_millisecond() + USED_UP_FOR_MS)
                .expect("half an hour from now is a time")
        });
        assembly.rotation().mark_used_up(&entry.id, &used_up, until);
        // The used-up account's hold goes before the choice, which then
        // counts the agent nowhere.
        self.holding = None;
        let (choice, candidates) = assembly.take_account(&entry, &model).await.map_err(drop)?;
        let Some(Choice::Account(holding)) = choice else {
            return Err(());
        };
        let next = holding.account().clone();
        self.holding = Some(holding);
        self.unannounced = false;
        let selection = self.selection.clone();
        self.serve(&entry, selection, Some(&next)).await.map_err(drop)?;
        let label = |id: &CredentialId| {
            candidates
                .iter()
                .find(|candidate| candidate.id == *id)
                .map_or_else(|| id.to_string(), |candidate| candidate.label.clone())
        };
        let switch = AccountSwitch {
            from: label(&used_up),
            to: label(&next),
            window: quota.map(|(window, _)| window),
            resets_at: moment,
        };
        Ok((switch, agent_account(&next, entry.rotation())))
    }

    /// Makes the current runtime one of `entry`'s provider for `selection`
    /// and `account`: the one there is while the entry, the account and the
    /// model are unchanged, otherwise a new one, and the one it replaces is
    /// closed. A Claude Code agent that changes account so closes its CLI
    /// process and starts one with the other account's token.
    async fn serve(
        &mut self,
        entry: &ProviderEntry,
        selection: ModelSelection,
        account: Option<&CredentialId>,
    ) -> Result<(), ProviderFailure> {
        if let EntryCredential::Subscription { active: None, .. } = &entry.credential {
            // The runtime of an account removed meanwhile goes, as a switch
            // closes it.
            self.close_current().await;
            return Err(refused(
                demi_backend_providers::vault::accounts::NO_ACCOUNT,
                Some(ErrorCode::AuthMissing),
            ));
        }
        let scope = self.scope.clone();
        let assembly = &scope.services.assembly;
        let provider = match account {
            Some(account) if entry.active() != Some(account) => assembly
                .for_account(entry, account)
                .await
                .map_err(|error| refused(error.to_string(), None))?
                .ok_or_else(|| refused("The account is no longer available", None))?,
            _ => assembly
                .provider_for(entry)
                .await
                .map_err(|error| refused(error.to_string(), None))?,
        };
        self.prepares_ahead = provider.capabilities().process_host;
        let kept = self.current.as_ref().is_some_and(|current| {
            Arc::ptr_eq(&current.provider, &provider) && current.model == selection.model.id
        });
        if !kept {
            // A provider that runs a process gets it on the user's Cloud,
            // whatever the conversation's target (`claude-code.md` § Where it
            // runs).
            let runtime = if provider.capabilities().process_host {
                let work = ProcessWork::Conversation(scope.conversation.clone());
                let placement = CloudPlacement::placement(scope.shard.clone(), work);
                let env = RuntimeEnv {
                    http: scope.http.clone(),
                };
                assembly
                    .process_runtime(entry, account.or(entry.active()), env, placement)
                    .await
                    .map_err(|error| refused(error.to_string(), None))?
            } else {
                provider
                    .runtime(RuntimeEnv {
                        http: scope.http.clone(),
                    })
                    .map_err(|error| refused(error.to_string(), None))?
            };
            let ledger = Ledger {
                control: scope.services.control.clone(),
                user: scope.user.clone(),
                conversation: scope.conversation.clone(),
                provider: scope.provider.clone(),
            };
            let metered = MeteredRuntime::new(runtime, scope.rate_limit.clone(), ledger);
            self.close_current().await;
            self.current = Some(Current {
                provider,
                model: selection.model.id.clone(),
                runtime: Box::new(metered),
            });
        }
        self.selection = selection;
        Ok(())
    }

    async fn close_current(&mut self) {
        if let Some(mut current) = self.current.take() {
            current.runtime.close().await;
        }
    }
}

/// The account an agent took under `rotation`, as its session keeps it.
fn agent_account(account: &CredentialId, rotation: Rotation) -> AgentAccount {
    AgentAccount {
        credential_id: account.to_string(),
        rotation,
    }
}

/// The failure of a request no account of `entry` can serve: every one is
/// used up, and the turn ends as it would with Manual
/// (`providers.md` § Account rotation).
fn all_used_up(entry: &ProviderEntry, candidates: &[Candidate]) -> ProviderFailure {
    let message = match candidates.len() {
        0 | 1 => format!("{} has reached its usage limit", entry.label),
        _ => format!("Every account of {} has reached its usage limit", entry.label),
    };
    let code = USAGE_LIMIT.parse::<ErrorCode>().unwrap_or_else(|never| match never {});
    refused(message, Some(code))
}

/// A used-up account's failure code.
const USAGE_LIMIT: &str = "usage_limit";

/// Whether `failure` says the account is used up.
fn uses_up(failure: &ProviderFailure) -> bool {
    failure.code.as_ref().is_some_and(|code| code.as_str() == USAGE_LIMIT)
}

impl ProviderRuntime for ConversationRuntime {
    /// Resolves the entry, then runs the request on the runtime that serves
    /// it. A request whose token is cancelled while the entry is read ends
    /// without an event, as every cancelled run does. Under rotation, a
    /// usage limit before anything of the run reached the agent moves the
    /// agent to another account and sends the same request again at once,
    /// which is no retry of the agent's (`providers.md` § Account rotation).
    fn run(&mut self, request: InferenceRequest) -> ProviderRun<'_> {
        Box::pin(async_stream::stream! {
            let cancel = request.cancel.clone();
            let admitted = tokio::select! {
                admitted = self.admit(&request) => admitted,
                () = cancel.cancelled() => return,
            };
            let admitted = match admitted {
                Ok(admitted) => admitted,
                Err(failure) => {
                    yield ProviderEvent::Error(failure);
                    return;
                }
            };
            if let Some(account) = admitted.announce {
                yield ProviderEvent::Account(account);
            }
            let request = InferenceRequest {
                output_limit: admitted.output_limit,
                ..request
            };
            loop {
                let mut used_up = None;
                {
                    let Some(current) = self.current.as_mut() else {
                        unreachable!("an admitted request has its runtime");
                    };
                    let mut events = current.runtime.run(request.clone());
                    let mut shown = false;
                    while let Some(event) = events.next().await {
                        match event {
                            ProviderEvent::Error(failure)
                                if admitted.rotates && !shown && uses_up(&failure) =>
                            {
                                used_up = Some(failure);
                            }
                            event => {
                                shown = true;
                                yield event;
                            }
                        }
                    }
                }
                let Some(failure) = used_up else {
                    return;
                };
                let switched = tokio::select! {
                    switched = self.switch_account(&failure) => switched,
                    () = cancel.cancelled() => return,
                };
                match switched {
                    Ok((switch, account)) => {
                        yield ProviderEvent::AccountSwitch(switch);
                        yield ProviderEvent::Account(Some(account));
                    }
                    Err(()) => {
                        yield ProviderEvent::Error(failure);
                        return;
                    }
                }
            }
        })
    }

    fn fresh(&self) -> Box<dyn ProviderRuntime> {
        // A child takes an account of its own at its first request.
        Box::new(Self {
            scope: self.scope.clone(),
            selection: self.selection.clone(),
            current: self.current.as_ref().map(|current| Current {
                provider: current.provider.clone(),
                model: current.model.clone(),
                runtime: current.runtime.fresh(),
            }),
            prepares_ahead: self.prepares_ahead,
            holding: None,
            unannounced: false,
        })
    }

    fn prepares_ahead(&self) -> bool {
        self.prepares_ahead
    }

    fn close(&mut self) -> LocalBoxFuture<'_, ()> {
        Box::pin(async move {
            self.holding = None;
            self.close_current().await;
        })
    }

    /// Resolves the entry as the request will, so a model or an account
    /// picked meanwhile gets its runtime now, and lets that runtime ready
    /// itself. An entry that cannot serve readies nothing: the request
    /// says why.
    fn prepare(&mut self, request: InferenceRequest, ahead: Ahead) -> LocalBoxFuture<'_, ()> {
        Box::pin(async move {
            let admitted = match self.admit(&request).await {
                Ok(admitted) => admitted,
                Err(failure) => {
                    tracing::debug!(
                        conversation = %self.scope.conversation,
                        "nothing was readied ahead: {}",
                        failure.message
                    );
                    return;
                }
            };
            if let Some(current) = &mut self.current {
                let request = InferenceRequest {
                    output_limit: admitted.output_limit,
                    ..request
                };
                current.runtime.prepare(request, ahead).await;
            }
        })
    }

    /// The limits of the runtime that serves the session's requests; none
    /// once the entry is gone, since its requests then fail before any
    /// vendor.
    fn request_limits(&self, model: &Model) -> RequestLimits {
        match &self.current {
            Some(current) => current.runtime.request_limits(model),
            None => RequestLimits::default(),
        }
    }
}

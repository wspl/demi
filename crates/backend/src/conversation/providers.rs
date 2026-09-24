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
use std::rc::Rc;
use std::sync::Arc;

use demi_agent::{ProviderResolver, ResolveError};
use demi_core::{Model, ModelSelection, NodeId};
use demi_provider::{
    ErrorCode, InferenceRequest, Provider, ProviderEvent, ProviderFailure, ProviderRun, ProviderRuntime, RuntimeEnv,
};
use demi_web_api::ids::{ConversationId, ProviderId, UserId};
use futures_util::future::LocalBoxFuture;
use futures_util::{StreamExt as _, stream};

use super::conversation_of;
use crate::backend::Services;
use crate::llm::catalog::configured_selection;
use crate::usage::meter::{Ledger, MeteredRuntime};
use crate::usage::rate_limit::RequestRateLimit;
use crate::vault::entries::{EntryCredential, ProviderEntry};

/// Where the user's sessions get their runtimes.
pub(super) struct ConversationProviders {
    user: UserId,
    services: Arc<Services>,
    /// The shard's HTTP client, which its runtimes send with.
    http: reqwest::Client,
    rate_limit: Rc<RefCell<RequestRateLimit>>,
}

impl ConversationProviders {
    pub(super) fn new(
        user: UserId,
        services: Arc<Services>,
        http: reqwest::Client,
        rate_limit: Rc<RefCell<RequestRateLimit>>,
    ) -> Self {
        Self {
            user,
            services,
            http,
            rate_limit,
        }
    }
}

impl ProviderResolver for ConversationProviders {
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
            let provider = ProviderId::try_from(model.provider_id.as_str()).map_err(|_| unknown())?;
            let scope = Rc::new(Scope {
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
            };
            runtime
                .serve(&entry, model.clone())
                .await
                .map_err(|failure| ResolveError::Failed(failure.message))?;
            Ok(Box::new(runtime) as Box<dyn ProviderRuntime>)
        })
    }
}

/// Whose requests a session's runtime makes, and what it builds its
/// runtimes with.
struct Scope {
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
}

/// The runtime that serves the session's requests, with what it was built
/// for.
struct Current {
    provider: Arc<dyn Provider>,
    model: String,
    runtime: Box<dyn ProviderRuntime>,
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
    /// and a subscription entry must have an account. Answers the request's
    /// output limit, which the entry's configured model decides when it has
    /// a configured list.
    async fn admit(&mut self, request: &InferenceRequest) -> Result<Option<NonZeroU32>, ProviderFailure> {
        let scope = self.scope.clone();
        let read = scope.services.vault.visible(&scope.user, &scope.provider).await;
        let entry = match read {
            Ok(Some(entry)) => entry,
            Ok(None) => {
                // The entry is gone: its runtime has nothing left to serve.
                self.close_current().await;
                return Err(refused(
                    format!("Provider \"{}\" is no longer available to this conversation", scope.provider),
                    None,
                ));
            }
            Err(error) => return Err(refused(format!("The provider entry could not be read: {error}"), None)),
        };
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
        self.serve(&entry, requested).await?;
        Ok(self.selection.model.output_limit.and_then(NonZeroU32::new))
    }

    /// Makes the current runtime one of `entry`'s provider for `requested`:
    /// the one there is while the entry and the model are unchanged,
    /// otherwise a new one, and the one it replaces is closed.
    async fn serve(&mut self, entry: &ProviderEntry, requested: ModelSelection) -> Result<(), ProviderFailure> {
        if let EntryCredential::Subscription { active: None } = &entry.credential {
            return Err(refused("No subscription account configured", Some(ErrorCode::AuthMissing)));
        }
        let selection = configured_selection(entry, requested).map_err(|error| refused(error.to_string(), None))?;
        let scope = self.scope.clone();
        let provider = scope
            .services
            .assembly
            .provider_for(entry)
            .await
            .map_err(|error| refused(error.to_string(), None))?;
        let kept = self
            .current
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(&current.provider, &provider) && current.model == selection.model.id);
        if !kept {
            let runtime = provider
                .runtime(RuntimeEnv {
                    http: scope.http.clone(),
                })
                .map_err(|error| refused(error.to_string(), None))?;
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

impl ProviderRuntime for ConversationRuntime {
    /// Resolves the entry, then runs the request on the runtime that serves
    /// it. A request whose token is cancelled while the entry is read ends
    /// without an event, as every cancelled run does.
    fn run(&mut self, request: InferenceRequest) -> ProviderRun<'_> {
        stream::once(async move {
            let cancel = request.cancel.clone();
            let admitted = tokio::select! {
                admitted = self.admit(&request) => admitted,
                () = cancel.cancelled() => return stream::empty().boxed_local(),
            };
            match admitted {
                Ok(output_limit) => {
                    let request = InferenceRequest { output_limit, ..request };
                    match self.current.as_mut() {
                        Some(current) => current.runtime.run(request),
                        None => unreachable!("an admitted request has its runtime"),
                    }
                }
                Err(failure) => stream::iter([ProviderEvent::Error(failure)]).boxed_local(),
            }
        })
        .flatten()
        .boxed_local()
    }

    fn fresh(&self) -> Box<dyn ProviderRuntime> {
        Box::new(Self {
            scope: self.scope.clone(),
            selection: self.selection.clone(),
            current: self.current.as_ref().map(|current| Current {
                provider: current.provider.clone(),
                model: current.model.clone(),
                runtime: current.runtime.fresh(),
            }),
        })
    }

    fn close(&mut self) -> LocalBoxFuture<'_, ()> {
        Box::pin(self.close_current())
    }
}

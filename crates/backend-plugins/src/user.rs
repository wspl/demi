//! One user's plugins on the user's shard (`plugins.md` § The plugin
//! host): an instance of every plugin, made when the shard starts, the
//! command set they serve, their page states and page calls, and the
//! answers to their port operations. The plugin host answers values and
//! changes itself; what reaches a conversation's Hosts or the user's
//! exposes goes to the product, which owns the conversation's host access.

use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;

use demi_backend_database::StorageError;
use demi_backend_database::control::ControlService;
use demi_backend_page_sync::{Part, UserMarks};
use demi_command_declarations::NativeOperation;
use demi_host_interface::{CommandSet, GroupBuilder, PortError, RpcPort};
use demi_plugin_interface::{
    CallKind, ConversationHost, ExposeList, ExposeRecord, Plugin, PluginError, PluginPort,
    PluginTransport, PortAnswer, PortFailure, PortMessage, PortRefusal, Reply, Request, Scope,
    StoredValue,
};
use demi_web_api_protocol::ids::{ConversationId, DeviceId, ExposeId, UserId};
use futures_util::future::LocalBoxFuture;
use serde_json::{Map, Value};
use tokio_util::sync::CancellationToken;

use crate::commands::compose;
use crate::{Registry, RegistryError};

/// What the product does for the plugins' port: the operations that reach
/// a conversation's Hosts through its host access, and the user's exposes.
/// The user's shard implements it.
pub trait ProductPort {
    /// Runs `operation` once on the conversation's main Host for the user,
    /// waking it or not as `kind` says, and answers its JSON result.
    fn package_call<'a>(
        &'a self,
        conversation: &'a ConversationId,
        operation: &'a NativeOperation,
        args: Map<String, Value>,
        kind: CallKind,
        cancel: &'a CancellationToken,
    ) -> LocalBoxFuture<'a, Result<Value, PortFailure>>;

    fn conversation_hosts<'a>(
        &'a self,
        conversation: &'a ConversationId,
    ) -> LocalBoxFuture<'a, Result<Vec<ConversationHost>, PortFailure>>;

    fn exposes(&self) -> LocalBoxFuture<'_, Result<ExposeList, PortFailure>>;

    fn create_expose(
        &self,
        device: DeviceId,
        address: String,
        lifetime: u64,
    ) -> LocalBoxFuture<'_, Result<ExposeRecord, PortFailure>>;

    fn renew_expose(
        &self,
        expose: ExposeId,
        lifetime: u64,
    ) -> LocalBoxFuture<'_, Result<ExposeRecord, PortFailure>>;

    fn remove_expose(&self, expose: ExposeId) -> LocalBoxFuture<'_, Result<(), PortFailure>>;
}

/// A page's call of a plugin method.
#[derive(Debug, Clone)]
pub struct PageCall {
    pub plugin: String,
    pub method: String,
    /// The JSON body, which the method's schema checks.
    pub params: Value,
    /// The conversation of the route, for a method of the conversation
    /// scope.
    pub conversation: Option<ConversationId>,
}

/// Why a page call did not answer.
#[derive(Debug, thiserror::Error)]
pub enum PageCallError {
    #[error("No plugin \"{0}\"")]
    UnknownPlugin(String),
    #[error("The plugin \"{plugin}\" has no method \"{method}\" here")]
    UnknownMethod { plugin: String, method: String },
    #[error("{0}")]
    InvalidParams(String),
    #[error(transparent)]
    Plugin(#[from] PluginError),
}

/// What every request of the user's plugins shares.
pub(crate) struct Shared {
    registry: Arc<Registry>,
    pub(crate) user: UserId,
    instances: Vec<Rc<dyn Plugin>>,
    product: Rc<dyn ProductPort>,
    control: ControlService,
    marks: UserMarks,
}

impl Shared {
    /// Sends `request` to plugin `plugin`, with a port for the request's
    /// conversation and, for a command, the call's rpc port.
    pub(crate) async fn request(
        self: &Rc<Self>,
        plugin: usize,
        request: Request,
        conversation: Option<ConversationId>,
        rpc: Option<RpcPort>,
        cancel: CancellationToken,
    ) -> Result<Reply, PluginError> {
        let transport = RequestPort {
            shared: self.clone(),
            plugin,
            conversation,
            rpc,
            cancel: cancel.clone(),
        };
        let port = PluginPort::new(Rc::new(transport), cancel);
        self.instances[plugin].call(request, port).await
    }

    fn plugin_id(&self, plugin: usize) -> String {
        self.registry.plugins[plugin].id().as_str().to_owned()
    }
}

/// One user's plugins. Cloning it is cheap.
#[derive(Clone)]
pub struct UserPlugins(Rc<Shared>);

impl UserPlugins {
    /// An instance of every plugin of `registry` for `user`, whose port
    /// reaches `product`, the user's values in `control` and the user's
    /// pages through `marks`.
    pub fn new(
        registry: Arc<Registry>,
        user: UserId,
        product: Rc<dyn ProductPort>,
        control: ControlService,
        marks: UserMarks,
    ) -> Self {
        let instances = registry
            .plugins
            .iter()
            .map(|registered| registered.factory.instance())
            .collect();
        Self(Rc::new(Shared {
            registry,
            user,
            instances,
            product,
            control,
            marks,
        }))
    }

    /// The command set every node of the user's conversations starts from:
    /// the plugins' groups under `demi` beside the product's `product`
    /// groups, and the plugins' roots.
    pub fn commands(&self, product: Vec<GroupBuilder>) -> Result<CommandSet, RegistryError> {
        compose(&self.0.registry, Some(&self.0), product)
    }

    /// The state of each plugin with a page, by id, for the product state.
    pub async fn page_states(&self) -> Result<BTreeMap<String, Value>, PluginError> {
        let mut states = BTreeMap::new();
        for (index, registered) in self.0.registry.plugins.iter().enumerate() {
            if registered
                .page
                .as_ref()
                .is_some_and(|page| page.state.is_some())
            {
                let state = self.state_of(index).await?;
                states.insert(registered.id().as_str().to_owned(), state);
            }
        }
        Ok(states)
    }

    /// The page state of `plugin`; none for a plugin whose page has none.
    pub async fn page_state(&self, plugin: &str) -> Result<Option<Value>, PluginError> {
        match self.0.registry.plugin(plugin) {
            Some((index, registered))
                if registered
                    .page
                    .as_ref()
                    .is_some_and(|page| page.state.is_some()) =>
            {
                self.state_of(index).await.map(Some)
            }
            _ => Ok(None),
        }
    }

    async fn state_of(&self, plugin: usize) -> Result<Value, PluginError> {
        let request = Request::PageState {
            user: self.0.user.clone(),
        };
        let reply = self
            .0
            .request(plugin, request, None, None, CancellationToken::new())
            .await?;
        match reply {
            Reply::State { state } => Ok(state),
            reply => Err(PluginError::failed(format!(
                "the plugin answered its page state with {reply:?}"
            ))),
        }
    }

    /// Calls a page method: the plugin has it for the call's scope, and
    /// its parameters fit its schema.
    pub async fn page_call(
        &self,
        call: PageCall,
        cancel: CancellationToken,
    ) -> Result<Value, PageCallError> {
        let Some((index, registered)) = self.0.registry.plugin(&call.plugin) else {
            return Err(PageCallError::UnknownPlugin(call.plugin));
        };
        let scope = match call.conversation {
            Some(_) => Scope::Conversation,
            None => Scope::User,
        };
        let method = registered.page.as_ref().and_then(|page| {
            page.methods
                .iter()
                .find(|method| method.name == call.method && method.scope == scope)
        });
        let Some(method) = method else {
            return Err(PageCallError::UnknownMethod {
                plugin: call.plugin,
                method: call.method,
            });
        };
        method
            .params
            .check(&call.params)
            .map_err(PageCallError::InvalidParams)?;
        let Value::Object(params) = call.params else {
            return Err(PageCallError::InvalidParams(
                "the parameters are not an object".into(),
            ));
        };
        let request = Request::PageCall {
            user: self.0.user.clone(),
            method: call.method,
            params,
            conversation: call.conversation.clone(),
        };
        let reply = self
            .0
            .request(index, request, call.conversation, None, cancel)
            .await?;
        match reply {
            Reply::Result { result } => Ok(result),
            reply => Err(PluginError::failed(format!(
                "the plugin answered a page call with {reply:?}"
            ))
            .into()),
        }
    }
}

/// One request's port: the call's rpc port for a command, the user's
/// values and pages, and the product's operations for the request's
/// conversation.
struct RequestPort {
    shared: Rc<Shared>,
    plugin: usize,
    conversation: Option<ConversationId>,
    rpc: Option<RpcPort>,
    cancel: CancellationToken,
}

impl RequestPort {
    async fn answer(&self, message: PortMessage) -> Result<PortAnswer, PortFailure> {
        let shared = &self.shared;
        let product = &*shared.product;
        let answer = match message {
            PortMessage::Rpc { request } => {
                let Some(rpc) = &self.rpc else {
                    return Err(PortError::Unexpected {
                        asked: "rpc",
                        answered: "no rpc port: the request is not a command",
                    }
                    .into());
                };
                PortAnswer::Rpc {
                    response: rpc.forward(request).await?,
                }
            }
            PortMessage::ReadValue { key } => {
                let value = shared
                    .control
                    .plugin_value(shared.user.clone(), shared.plugin_id(self.plugin), key)
                    .await
                    .map_err(storage)?;
                PortAnswer::Value {
                    value: value.map(stored),
                }
            }
            PortMessage::ListValues => {
                let values = shared
                    .control
                    .plugin_values(shared.user.clone(), shared.plugin_id(self.plugin))
                    .await
                    .map_err(storage)?;
                PortAnswer::Values {
                    values: values
                        .into_iter()
                        .map(|(key, value)| (key, stored(value)))
                        .collect(),
                }
            }
            PortMessage::WriteValue {
                key,
                value,
                revision,
            } => {
                let written = shared
                    .control
                    .write_plugin_value(
                        shared.user.clone(),
                        shared.plugin_id(self.plugin),
                        key,
                        value,
                        revision,
                    )
                    .await
                    .map_err(storage)?;
                let revision = written.ok_or(PortFailure::Refused(PortRefusal::Conflict))?;
                PortAnswer::Written { revision }
            }
            PortMessage::Changed => {
                shared
                    .marks
                    .mark(Part::Plugin(shared.plugin_id(self.plugin)));
                PortAnswer::Done
            }
            PortMessage::PackageCall {
                operation,
                args,
                kind,
            } => {
                let registered = &shared.registry.plugins[self.plugin];
                if !registered.packages.contains(&operation.package) {
                    return Err(PortError::Unexpected {
                        asked: "package_call",
                        answered: "a package the plugin's commands do not bind",
                    }
                    .into());
                }
                let conversation = self.conversation()?;
                let result = product
                    .package_call(conversation, &operation, args, kind, &self.cancel)
                    .await?;
                PortAnswer::Called { result }
            }
            PortMessage::ConversationHosts => PortAnswer::Hosts {
                hosts: product.conversation_hosts(self.conversation()?).await?,
            },
            PortMessage::ListExposes => PortAnswer::Exposes {
                list: product.exposes().await?,
            },
            PortMessage::CreateExpose {
                device,
                address,
                lifetime,
            } => PortAnswer::Expose {
                expose: product.create_expose(device, address, lifetime).await?,
            },
            PortMessage::RenewExpose { expose, lifetime } => PortAnswer::Expose {
                expose: product.renew_expose(expose, lifetime).await?,
            },
            PortMessage::RemoveExpose { expose } => {
                product.remove_expose(expose).await?;
                PortAnswer::Done
            }
        };
        Ok(answer)
    }

    fn conversation(&self) -> Result<&ConversationId, PortFailure> {
        self.conversation
            .as_ref()
            .ok_or(PortFailure::Refused(PortRefusal::NoConversation))
    }
}

impl PluginTransport for RequestPort {
    fn request(&self, message: PortMessage) -> LocalBoxFuture<'_, Result<PortAnswer, PortError>> {
        Box::pin(async move {
            match self.answer(message).await {
                Ok(answer) => Ok(answer),
                Err(PortFailure::Refused(refusal)) => Ok(PortAnswer::Refused { refusal }),
                Err(PortFailure::Port(error)) => Err(error),
            }
        })
    }
}

fn stored(value: demi_backend_database::plugin_values::PluginValue) -> StoredValue {
    StoredValue {
        value: value.document,
        revision: value.revision,
    }
}

/// A storage failure as the port reports it: the operation failed, which a
/// plugin cannot cause.
fn storage(error: StorageError) -> PortFailure {
    PortFailure::Port(PortError::Failed(error.to_string()))
}

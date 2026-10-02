//! The plugin's side of a request (`plugins.md` § The contract): every
//! operation is one message and one answer, carried by a transport the
//! plugin host supplies. A refusal is an answer too, so it crosses a wire as
//! data.

use std::collections::BTreeMap;
use std::rc::Rc;

use demi_command_declarations::NativeOperation;
use demi_host_interface::{PortError, PortRequest, PortResponse, PortTransport, RpcPort};
use demi_shared_types::Timestamp;
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::exposes::ExposeAddress;
use demi_web_api_protocol::ids::{DeviceId, ExposeId};
use futures_util::future::LocalBoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tokio_util::sync::{CancellationToken, WaitForCancellationFuture};

/// One operation a plugin asks of Demi.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum PortMessage {
    /// An operation of a command request's rpc port: its IO and the
    /// invoking node's command storage.
    Rpc { request: PortRequest },
    /// The plugin's value `key` for the user.
    ReadValue { key: String },
    /// Every value of the plugin's for the user.
    ListValues,
    /// Writes `key` if its revision is still `revision`; none for a value
    /// that does not exist yet.
    WriteValue {
        key: String,
        value: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        revision: Option<u64>,
    },
    /// The plugin's part of the user's product state changed.
    Changed,
    /// Runs one operation of a package the plugin's commands bind, on the
    /// request's conversation's main Host.
    PackageCall {
        operation: NativeOperation,
        args: Map<String, Value>,
        kind: CallKind,
    },
    /// The request's conversation's main and attached Hosts.
    ConversationHosts,
    /// The user's live exposes, soonest expiry first.
    ListExposes,
    /// A new expose of `address` on the user's `device`, for `lifetime`
    /// seconds.
    CreateExpose {
        device: DeviceId,
        address: String,
        lifetime: u64,
    },
    /// Moves the expose's expiry to `lifetime` seconds from now.
    RenewExpose { expose: ExposeId, lifetime: u64 },
    /// Destroys the expose at once.
    RemoveExpose { expose: ExposeId },
}

/// The answer to one [`PortMessage`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum PortAnswer {
    Rpc {
        response: PortResponse,
    },
    Value {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<StoredValue>,
    },
    Values {
        values: BTreeMap<String, StoredValue>,
    },
    /// The value's new revision.
    Written {
        revision: u64,
    },
    /// The operation is done and answers nothing.
    Done,
    /// A package call's JSON result.
    Called {
        result: Value,
    },
    Hosts {
        hosts: Vec<ConversationHost>,
    },
    Exposes {
        list: ExposeList,
    },
    Expose {
        expose: ExposeRecord,
    },
    Refused {
        refusal: PortRefusal,
    },
}

impl PortAnswer {
    fn name(&self) -> &'static str {
        match self {
            Self::Rpc { .. } => "rpc",
            Self::Value { .. } => "value",
            Self::Values { .. } => "values",
            Self::Written { .. } => "written",
            Self::Done => "done",
            Self::Called { .. } => "called",
            Self::Hosts { .. } => "hosts",
            Self::Exposes { .. } => "exposes",
            Self::Expose { .. } => "expose",
            Self::Refused { .. } => "refused",
        }
    }
}

/// A stored value and the revision a write of it names.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StoredValue {
    pub value: Value,
    pub revision: u64,
}

/// What a package call does on the Host, which decides whether it wakes a
/// stopped Cloud and whether it is activity (`resource-lifecycle.md`
/// § Activity).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallKind {
    /// Work the user starts, such as opening a tab: it wakes a stopped
    /// Cloud.
    Starts,
    /// An operation on what runs there, such as closing a tab: activity,
    /// but a stopped Cloud is refused rather than woken.
    Operates,
    /// A look, such as listing the tabs: a stopped Cloud is refused, and
    /// the look is no activity.
    Looks,
}

/// A Host of the request's conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConversationHost {
    /// Its name as `demi host list` shows it.
    pub name: String,
    pub device: DeviceId,
    pub role: HostRole,
    pub online: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostRole {
    Main,
    Attached,
}

/// The user's live exposes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExposeList {
    /// Whether the instance has an expose domain; without one there are no
    /// exposes.
    pub available: bool,
    /// When Demi listed them, by the clock their expiries are read by.
    pub listed_at: Timestamp,
    /// Soonest expiry first.
    pub exposes: Vec<ExposeRecord>,
}

/// An expose as the port shows it (`expose.md` § The expose record).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExposeRecord {
    pub id: ExposeId,
    pub device: DeviceId,
    /// The device's name, the Cloud's as `Cloud`.
    pub device_name: String,
    pub address: ExposeAddress,
    pub url: String,
    pub created_at: Timestamp,
    pub expires_at: Timestamp,
}

/// Why Demi refused a port operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum PortRefusal {
    /// The conversation's host access refused, as its routes would: a page
    /// call that passes it on answers with its code and status.
    #[error("{message}")]
    Host {
        code: ErrorCode,
        status: u16,
        message: String,
    },
    /// The package operation exited nonzero, with what it wrote to its
    /// standard error.
    #[error("the operation failed: {stderr}")]
    Operation { stderr: String },
    /// Another write of the value came first.
    #[error("the value changed since it was read")]
    Conflict,
    /// An expose operation was refused.
    #[error("{message}")]
    Expose {
        reason: ExposeRefusal,
        message: String,
    },
    /// The operation needs a conversation, and the request has none.
    #[error("the request has no conversation")]
    NoConversation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExposeRefusal {
    /// The instance has no expose domain.
    Unavailable,
    /// The address is not `host:port` or a port.
    InvalidAddress,
    /// The device is not the user's.
    DeviceNotFound,
    /// The device's runner is not connected, or its Cloud is not running.
    DeviceOffline,
    /// The user has no live expose of that id.
    NotFound,
}

/// Why a typed port operation did not answer.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PortFailure {
    #[error(transparent)]
    Port(#[from] PortError),
    #[error(transparent)]
    Refused(PortRefusal),
}

/// What carries a port's messages: the plugin host's answers in process, a
/// wire to a plugin process, or a test's.
pub trait PluginTransport {
    fn request(&self, message: PortMessage) -> LocalBoxFuture<'_, Result<PortAnswer, PortError>>;
}

/// The plugin's side of one request.
#[derive(Clone)]
pub struct PluginPort {
    transport: Rc<dyn PluginTransport>,
    cancel: CancellationToken,
}

impl PluginPort {
    pub fn new(transport: Rc<dyn PluginTransport>, cancel: CancellationToken) -> Self {
        Self { transport, cancel }
    }

    /// A command request's rpc port, whose operations travel as
    /// [`PortMessage::Rpc`].
    pub fn rpc(&self) -> RpcPort {
        RpcPort::new(
            Rc::new(RpcMessages(self.transport.clone())),
            self.cancel.clone(),
        )
    }

    /// The loopback's JSON round trip wraps the port's own transport.
    #[cfg(feature = "testing")]
    pub(crate) fn transport(&self) -> &Rc<dyn PluginTransport> {
        &self.transport
    }

    #[cfg(feature = "testing")]
    pub(crate) fn cancellation(&self) -> &CancellationToken {
        &self.cancel
    }

    /// Completes once the request is stopped.
    pub fn cancelled(&self) -> WaitForCancellationFuture<'_> {
        self.cancel.cancelled()
    }

    /// Sends `message`; a refusal is the operation's failure.
    async fn ask(&self, message: PortMessage) -> Result<PortAnswer, PortFailure> {
        match self.transport.request(message).await? {
            PortAnswer::Refused { refusal } => Err(PortFailure::Refused(refusal)),
            answer => Ok(answer),
        }
    }

    pub async fn value(&self, key: impl Into<String>) -> Result<Option<StoredValue>, PortFailure> {
        match self.ask(PortMessage::ReadValue { key: key.into() }).await? {
            PortAnswer::Value { value } => Ok(value),
            answer => Err(unexpected("read_value", &answer)),
        }
    }

    pub async fn values(&self) -> Result<BTreeMap<String, StoredValue>, PortFailure> {
        match self.ask(PortMessage::ListValues).await? {
            PortAnswer::Values { values } => Ok(values),
            answer => Err(unexpected("list_values", &answer)),
        }
    }

    /// Writes `key` if it is still at `revision`, none for a new value, and
    /// answers its new revision; [`PortRefusal::Conflict`] when another
    /// write came first.
    pub async fn write_value(
        &self,
        key: impl Into<String>,
        value: Value,
        revision: Option<u64>,
    ) -> Result<u64, PortFailure> {
        let message = PortMessage::WriteValue {
            key: key.into(),
            value,
            revision,
        };
        match self.ask(message).await? {
            PortAnswer::Written { revision } => Ok(revision),
            answer => Err(unexpected("write_value", &answer)),
        }
    }

    /// Marks the plugin's part of the user's product state as changed.
    pub async fn changed(&self) -> Result<(), PortFailure> {
        self.done("changed", PortMessage::Changed).await
    }

    pub async fn package_call(
        &self,
        operation: NativeOperation,
        args: Map<String, Value>,
        kind: CallKind,
    ) -> Result<Value, PortFailure> {
        let message = PortMessage::PackageCall {
            operation,
            args,
            kind,
        };
        match self.ask(message).await? {
            PortAnswer::Called { result } => Ok(result),
            answer => Err(unexpected("package_call", &answer)),
        }
    }

    pub async fn conversation_hosts(&self) -> Result<Vec<ConversationHost>, PortFailure> {
        match self.ask(PortMessage::ConversationHosts).await? {
            PortAnswer::Hosts { hosts } => Ok(hosts),
            answer => Err(unexpected("conversation_hosts", &answer)),
        }
    }

    pub async fn exposes(&self) -> Result<ExposeList, PortFailure> {
        match self.ask(PortMessage::ListExposes).await? {
            PortAnswer::Exposes { list } => Ok(list),
            answer => Err(unexpected("list_exposes", &answer)),
        }
    }

    pub async fn create_expose(
        &self,
        device: DeviceId,
        address: String,
        lifetime: u64,
    ) -> Result<ExposeRecord, PortFailure> {
        let message = PortMessage::CreateExpose {
            device,
            address,
            lifetime,
        };
        self.expose("create_expose", message).await
    }

    pub async fn renew_expose(
        &self,
        expose: ExposeId,
        lifetime: u64,
    ) -> Result<ExposeRecord, PortFailure> {
        let message = PortMessage::RenewExpose { expose, lifetime };
        self.expose("renew_expose", message).await
    }

    pub async fn remove_expose(&self, expose: ExposeId) -> Result<(), PortFailure> {
        self.done("remove_expose", PortMessage::RemoveExpose { expose })
            .await
    }

    async fn expose(
        &self,
        asked: &'static str,
        message: PortMessage,
    ) -> Result<ExposeRecord, PortFailure> {
        match self.ask(message).await? {
            PortAnswer::Expose { expose } => Ok(expose),
            answer => Err(unexpected(asked, &answer)),
        }
    }

    async fn done(&self, asked: &'static str, message: PortMessage) -> Result<(), PortFailure> {
        match self.ask(message).await? {
            PortAnswer::Done => Ok(()),
            answer => Err(unexpected(asked, &answer)),
        }
    }
}

fn unexpected(asked: &'static str, answer: &PortAnswer) -> PortFailure {
    PortFailure::Port(PortError::Unexpected {
        asked,
        answered: answer.name(),
    })
}

/// An rpc port's operations as port messages.
struct RpcMessages(Rc<dyn PluginTransport>);

impl PortTransport for RpcMessages {
    fn request(&self, request: PortRequest) -> LocalBoxFuture<'_, Result<PortResponse, PortError>> {
        Box::pin(async move {
            match self.0.request(PortMessage::Rpc { request }).await? {
                PortAnswer::Rpc { response } => Ok(response),
                answer => Err(PortError::Unexpected {
                    asked: "rpc",
                    answered: answer.name(),
                }),
            }
        })
    }
}

//! The plugin's side of a request (`plugins.md` § The contract): every
//! operation is one message and one answer, carried by a transport the
//! plugin host supplies.

use std::rc::Rc;

use demi_host_interface::{PortError, PortRequest, PortResponse, PortTransport, RpcPort};
use futures_util::future::LocalBoxFuture;
use serde::{Deserialize, Serialize};
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
    Rpc { response: PortResponse },
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

    pub(crate) fn transport(&self) -> &Rc<dyn PluginTransport> {
        &self.transport
    }

    pub(crate) fn cancellation(&self) -> &CancellationToken {
        &self.cancel
    }

    /// Completes once the request is stopped.
    pub fn cancelled(&self) -> WaitForCancellationFuture<'_> {
        self.cancel.cancelled()
    }
}

/// An rpc port's operations as port messages.
struct RpcMessages(Rc<dyn PluginTransport>);

impl PortTransport for RpcMessages {
    fn request(&self, request: PortRequest) -> LocalBoxFuture<'_, Result<PortResponse, PortError>> {
        Box::pin(async move {
            let PortAnswer::Rpc { response } = self.0.request(PortMessage::Rpc { request }).await?;
            Ok(response)
        })
    }
}

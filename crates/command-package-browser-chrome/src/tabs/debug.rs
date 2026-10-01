//! The way to a tab's debugging owner (`browser.md` § CDP commands and
//! events): the owner of the debugging connections of `cdp` commands, one
//! per calling agent, and of the events they record. The first `cdp` command
//! on the tab starts the owner; it ends with the tab. Closing the tab first
//! releases every connection, and a timeout's diagnostics name the other
//! agents whose connections are open.

use std::sync::OnceLock;

use serde_json::Value;
use tokio::sync::{mpsc, oneshot, watch};
use tokio_util::sync::CancellationToken;

use crate::driver::operation::{BrowserError, CONTROL_TIMEOUT, Result};

use crate::tabs::protocol::CdpEventsResult;

/// A tab's debugging owner, once a `cdp` command started it.
#[derive(Default)]
pub struct DebugSessions {
    owner: OnceLock<DebugOwner>,
}

impl DebugSessions {
    /// The owner, which `start` starts on the first call.
    pub fn owner(&self, start: impl FnOnce() -> DebugOwner) -> &DebugOwner {
        self.owner.get_or_init(start)
    }

    /// The other callers whose connections to the tab are open.
    pub fn other_callers(&self, caller: Option<u64>) -> Vec<u64> {
        let Some(owner) = self.owner.get() else {
            return Vec::new();
        };
        owner
            .callers
            .borrow()
            .iter()
            .filter(|owner| Some(**owner) != caller)
            .copied()
            .collect()
    }

    /// Closes every connection; once this returns, all are gone.
    pub async fn release(&self) -> Result<()> {
        match self.owner.get() {
            Some(owner) => owner.release().await,
            None => Ok(()),
        }
    }
}

/// The way to a running debugging owner.
pub struct DebugOwner {
    pub requests: mpsc::Sender<SessionsRequest>,
    /// The numbers of the agents with a connection open, for a timeout's
    /// diagnostics.
    pub callers: watch::Receiver<Vec<u64>>,
    /// Where the next recorded event goes, so a waiting reader learns of new
    /// events.
    pub recorded: watch::Receiver<u64>,
    /// Cancelled once the owner has closed every connection and ended.
    pub finished: CancellationToken,
}

pub enum SessionsRequest {
    /// The caller's connection, made if it has none.
    Connect {
        caller: u64,
        reply: oneshot::Sender<Result<DebugHandle>>,
    },
    /// Closes the caller's connection.
    Detach {
        caller: u64,
        reply: oneshot::Sender<Result<()>>,
    },
    Events {
        query: Query,
        reply: oneshot::Sender<Result<EventPage>>,
    },
    /// Closes every connection.
    Release { reply: oneshot::Sender<Result<()>> },
}

/// What a `cdp events` read asks for.
pub struct Query {
    pub after: Option<String>,
    pub limit: usize,
    pub methods: Option<Vec<String>>,
    pub target: String,
}

/// A page of recorded events. `wait` when it holds none after a cursor: a
/// read with a timeout then waits for the next event.
pub struct EventPage {
    pub result: CdpEventsResult,
    pub wait: bool,
}

impl DebugOwner {
    async fn ask<T>(
        &self,
        request: impl FnOnce(oneshot::Sender<Result<T>>) -> SessionsRequest,
    ) -> Result<T> {
        let (reply, answer) = oneshot::channel();
        self.requests
            .send(request(reply))
            .await
            .map_err(|_| BrowserError::Closed)?;
        // The owner drops its requests when the tab ends.
        answer.await.map_err(|_| BrowserError::Closed)?
    }

    /// The connection of the agent numbered `caller` to the tab, made if it
    /// has none.
    pub async fn connect(&self, caller: u64) -> Result<DebugHandle> {
        self.ask(|reply| SessionsRequest::Connect { caller, reply })
            .await
    }

    /// Closes `caller`'s connection and joins its cleanup.
    pub async fn detach(&self, caller: u64) -> Result<()> {
        self.ask(|reply| SessionsRequest::Detach { caller, reply })
            .await
    }

    pub async fn events(&self, query: Query) -> Result<EventPage> {
        self.ask(|reply| SessionsRequest::Events { query, reply })
            .await
    }

    /// Watches the recorded events from now on.
    pub fn recorded(&self) -> watch::Receiver<u64> {
        let mut recorded = self.recorded.clone();
        recorded.mark_unchanged();
        recorded
    }

    /// Closes every connection; once this returns, all are gone.
    async fn release(&self) -> Result<()> {
        match self.ask(|reply| SessionsRequest::Release { reply }).await {
            // The owner closes every connection as it ends with its tab.
            Err(BrowserError::Closed) => {
                let _unfinished =
                    tokio::time::timeout(CONTROL_TIMEOUT, self.finished.cancelled()).await;
                Ok(())
            }
            released => released,
        }
    }
}

/// The way to a connection's pump.
#[derive(Clone)]
pub struct DebugHandle {
    pub requests: mpsc::Sender<ConnectionRequest>,
}

pub enum ConnectionRequest {
    Send {
        method: String,
        params: Value,
        target: String,
        response: oneshot::Sender<Result<Value>>,
    },
    /// The targets the connection attached.
    Targets { reply: oneshot::Sender<Vec<String>> },
}

impl DebugHandle {
    pub async fn send(&self, method: &str, params: Value, target: &str) -> Result<Value> {
        let (response, result) = oneshot::channel();
        self.requests
            .send(ConnectionRequest::Send {
                method: method.into(),
                params,
                target: target.into(),
                response,
            })
            .await
            .map_err(|_| BrowserError::Closed)?;
        result.await.map_err(|_| BrowserError::Closed)?
    }

    pub async fn targets(&self) -> Result<Vec<String>> {
        let (reply, targets) = oneshot::channel();
        self.requests
            .send(ConnectionRequest::Targets { reply })
            .await
            .map_err(|_| BrowserError::Closed)?;
        targets.await.map_err(|_| BrowserError::Closed)
    }
}

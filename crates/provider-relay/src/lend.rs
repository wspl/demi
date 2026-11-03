//! The lender's end of a borrowed session: the socket's requests run on the
//! lent entry's runtime, and what crosses back is what the borrower may see.
//!
//! The session keeps the account its entry's rotation gave it, as a
//! conversation's session keeps it with its state, and names it in each of
//! its requests; the borrower never hears of it, nor of the account's
//! switches and quota. A failure crosses as its code alone, as its type
//! says (`demi_provider_common::ProviderFailure`), and is logged here
//! whole.

use demi_provider_common::{CallEvent, ErrorCode, ProviderEvent, ProviderFailure, ProviderRuntime};
use demi_shared_types::AgentAccount;
use futures_util::{Sink, SinkExt as _, Stream, StreamExt as _};
use tokio_util::sync::CancellationToken;

use crate::delta::Inbox;
use crate::wire::{ClientMessage, ServerMessage};

/// Whether `event` crosses: none of the account's or its quota's does.
fn lent(event: &ProviderEvent) -> bool {
    !matches!(event, ProviderEvent::Account(_) | ProviderEvent::AccountSwitch(_) | ProviderEvent::Call(CallEvent::Observed(_)))
}

/// Whether the lender lends a run, as its own settings say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lends {
    Yes,
    /// The provider is turned off.
    TurnedOff,
    /// The run's model is turned off.
    NotTheModel,
}

/// Serves the session of one socket until `incoming` ends or `outgoing`
/// closes, then closes `runtime`. `key` names the share key in the log.
///
/// `lends` says, before each run, whether the lender still lends the
/// provider and the run's model. A provider it turned off ends the session,
/// and the borrower, asking why, hears that it is off; a model it turned off
/// fails the run as one the provider does not offer. A setting `lends`
/// cannot read ends the session too, logged as the failure it is.
pub async fn serve<In, Out>(
    mut runtime: Box<dyn ProviderRuntime>,
    mut incoming: In,
    mut outgoing: Out,
    key: &str,
    mut lends: impl AsyncFnMut(&str) -> Result<Lends, String>,
) where
    In: Stream<Item = ClientMessage> + Unpin,
    Out: Sink<ServerMessage> + Unpin,
{
    let mut inbox = Inbox::default();
    let mut account: Option<AgentAccount> = None;
    'session: while let Some(message) = incoming.next().await {
        let (run, request) = match message {
            ClientMessage::Run { run, request } => (run, request),
            ClientMessage::Idle => {
                runtime.idle();
                continue;
            }
            // A run that ended before its cancel came.
            ClientMessage::Cancel { .. } => continue,
        };
        let cancel = CancellationToken::new();
        let mut request = match inbox.receive(request, cancel.clone()) {
            Ok(request) => request,
            Err(error) => {
                tracing::warn!(event = "relay.out_of_step", key, %error, "a borrowed session broke the relay's rules");
                break;
            }
        };
        match lends(&request.model_id).await {
            Ok(Lends::Yes) => {}
            Ok(Lends::TurnedOff) => {
                tracing::info!(event = "relay.turned_off", key, "a borrowed session ended: its provider is turned off");
                break;
            }
            Err(error) => {
                tracing::warn!(event = "relay.lends_unread", key, %error, "a borrowed session ended: what it lends could not be read");
                break;
            }
            Ok(Lends::NotTheModel) => {
                let failure = ProviderEvent::Error(ProviderFailure {
                    message: String::new(),
                    code: Some(ErrorCode::ModelUnavailable),
                    diagnostics: None,
                    retry_after: None,
                });
                let sent = outgoing.send(ServerMessage::Event { run, event: failure }).await.is_ok()
                    && outgoing.send(ServerMessage::End { run }).await.is_ok();
                if !sent {
                    break;
                }
                continue;
            }
        }
        request.account = account.clone();
        let mut events = runtime.run(request);
        let mut cancelled = false;
        loop {
            tokio::select! {
                event = events.next() => {
                    let Some(event) = event else { break };
                    if let ProviderEvent::Account(taken) = event {
                        account = taken;
                        continue;
                    }
                    if let ProviderEvent::Error(failure) = &event {
                        tracing::info!(
                            event = "relay.run_failed",
                            key,
                            code = failure.code.as_ref().map(|code| code.as_str()),
                            message = %failure.message,
                            "a borrowed run failed"
                        );
                    }
                    if !lent(&event) {
                        continue;
                    }
                    if outgoing.send(ServerMessage::Event { run, event }).await.is_err() {
                        break 'session;
                    }
                }
                message = incoming.next(), if !cancelled => match message {
                    Some(ClientMessage::Cancel { run: cancelled_run }) if cancelled_run == run => {
                        cancel.cancel();
                        cancelled = true;
                    }
                    Some(ClientMessage::Cancel { .. }) => {}
                    Some(ClientMessage::Idle) => {}
                    Some(ClientMessage::Run { .. }) => {
                        tracing::warn!(event = "relay.out_of_step", key, "a borrowed session sent a run during another");
                        break 'session;
                    }
                    None => break 'session,
                },
            }
        }
        drop(events);
        if outgoing.send(ServerMessage::End { run }).await.is_err() {
            break;
        }
    }
    runtime.close().await;
}

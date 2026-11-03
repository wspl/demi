//! The lender's end of a borrowed session: the socket's requests run on the
//! lent entry's runtime, and what crosses back is what the borrower may see.
//!
//! The session keeps the account its entry's rotation gave it, as a
//! conversation's session keeps it with its state, and names it in each of
//! its requests; the borrower never hears of it. A failure crosses as its
//! code alone, and is logged here whole.

use demi_provider_common::{ProviderEvent, ProviderRuntime};
use demi_shared_types::AgentAccount;
use futures_util::{Sink, SinkExt as _, Stream, StreamExt as _};
use tokio_util::sync::CancellationToken;

use crate::delta::Inbox;
use crate::wire::{ClientMessage, ServerMessage, WireEvent};

/// Serves the session of one socket until `incoming` ends or `outgoing`
/// closes, then closes `runtime`. `key` names the share key in the log.
pub async fn serve<In, Out>(mut runtime: Box<dyn ProviderRuntime>, mut incoming: In, mut outgoing: Out, key: &str)
where
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
                    let Some(event) = WireEvent::lent(event) else { continue };
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

//! A tab's debugging owner (`browser.md` § CDP commands and events): one
//! task per tab owns its debugging connections, one per calling agent, and
//! the events they record. Commands ask it for their connection and for
//! pages of the events; each connection's pump owns its socket and the
//! targets it attached, and answers the commands that use it. Everything it
//! sends and receives is validated against the pinned protocol.

use std::collections::HashMap;

use chromiumoxide::{
    cdp::browser_protocol::target::{
        AttachToTargetReturns, EventAttachedToTarget, EventDetachedFromTarget, SessionId, TargetId,
    },
    conn::Connection,
    types::{CallId, EventMessage, Message, Method, MethodId},
};
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot, watch};
use tokio_util::{
    sync::CancellationToken,
    task::{AbortOnDropHandle, TaskTracker},
};

use crate::driver::{
    history::Buffer,
    operation::{BrowserError, CONTROL_TIMEOUT, Result, after_cleanup},
};
use crate::tabs::{
    debug::{ConnectionRequest, DebugHandle, DebugOwner, EventPage, Query, SessionsRequest},
    environment::BrowserHandle,
    tab::BrowserTab,
};

use crate::cdp::catalog::catalog;
use crate::cdp::protocol::{CDP_BYTES, CDP_EVENTS, CdpEvent, CdpEventsResult};

/// Requests waiting for a tab's debugging owner or for a connection's pump;
/// a full queue holds back their senders.
const REQUESTS: usize = 16;
/// Events the pumps have read and the owner has not recorded yet; a full
/// queue holds back the pumps' reads.
const RECORDING: usize = 64;

/// Starts the debugging owner of `tab`; it ends with the tab.
pub(crate) fn start(tab: &BrowserTab) -> DebugOwner {
    let (requests, received) = mpsc::channel(REQUESTS);
    let (recording, recordings) = mpsc::channel(RECORDING);
    let (callers_sender, callers) = watch::channel(Vec::new());
    let (recorded_sender, recorded) = watch::channel(0);
    let finished = CancellationToken::new();
    let owner = SessionsOwner {
        browser: tab.browser().clone(),
        target: tab.page().target_id().clone(),
        ended: tab.ended().clone(),
        tasks: tab.state().tasks.clone(),
        connections: HashMap::new(),
        buffer: None,
        recording,
        callers: callers_sender,
        recorded: recorded_sender,
    };
    tab.state()
        .tasks
        .spawn(owner.run(received, recordings, finished.clone()));
    DebugOwner {
        requests,
        callers,
        recorded,
        finished,
    }
}

struct SessionsOwner {
    browser: BrowserHandle,
    target: TargetId,
    /// The tab's end.
    ended: CancellationToken,
    tasks: TaskTracker,
    /// Each calling agent's connection, by the agent's number.
    connections: HashMap<u64, DebugConnection>,
    /// The tab's recorded events, while a connection is open.
    buffer: Option<Buffer<CdpEvent>>,
    recording: mpsc::Sender<Recorded>,
    callers: watch::Sender<Vec<u64>>,
    recorded: watch::Sender<u64>,
}

/// What a connection's pump tells the owner.
enum Recorded {
    Event(CdpEvent),
    /// A pump ended on its own.
    Ended,
}

impl SessionsOwner {
    async fn run(
        mut self,
        mut requests: mpsc::Receiver<SessionsRequest>,
        mut recordings: mpsc::Receiver<Recorded>,
        finished: CancellationToken,
    ) {
        let _finished = finished.drop_guard();
        loop {
            tokio::select! {
                biased;
                _ = self.ended.cancelled() => break,
                recorded = recordings.recv() => match recorded {
                    Some(Recorded::Event(event)) => self.record(event),
                    // Its commands fail as ended until the caller detaches.
                    Some(Recorded::Ended) => self.publish_callers(),
                    None => break,
                },
                request = requests.recv() => match request {
                    Some(request) => self.request(request).await,
                    None => break,
                },
            }
        }
        if let Err(error) = self.close_all().await {
            tracing::warn!("a closed tab's debugging connections did not close cleanly: {error}");
        }
    }

    async fn request(&mut self, request: SessionsRequest) {
        match request {
            SessionsRequest::Connect { caller, reply } => {
                let connection = self.connect(caller).await;
                // A caller that left needs no answer.
                let _left = reply.send(connection);
            }
            SessionsRequest::Detach { caller, reply } => {
                let closed = match self.connections.remove(&caller) {
                    Some(connection) => connection.close().await,
                    None => Ok(()),
                };
                if self.connections.is_empty() {
                    self.buffer = None;
                }
                self.publish_callers();
                let _left = reply.send(closed);
            }
            SessionsRequest::Events { query, reply } => {
                let _left = reply.send(self.page(query));
            }
            SessionsRequest::Release { reply } => {
                let _left = reply.send(self.close_all().await);
            }
        }
    }

    async fn connect(&mut self, caller: u64) -> Result<DebugHandle> {
        if let Some(connection) = self.connections.get(&caller) {
            if let Some(reason) = connection.ended() {
                return Err(ended(&reason));
            }
            return Ok(connection.handle.clone());
        }
        if self.buffer.is_none() {
            self.buffer = Some(Buffer::new("cdp", CDP_EVENTS, CDP_BYTES)?);
        }
        let started = tokio::time::timeout(
            CONTROL_TIMEOUT,
            DebugConnection::start(
                &self.browser,
                &self.target,
                &self.ended,
                &self.tasks,
                self.recording.clone(),
            ),
        )
        .await
        .map_err(|_| BrowserError::Timeout)
        .and_then(std::convert::identity);
        match started {
            Ok(connection) => {
                let handle = connection.handle.clone();
                self.connections.insert(caller, connection);
                self.publish_callers();
                Ok(handle)
            }
            Err(error) => {
                if self.connections.is_empty() {
                    self.buffer = None;
                }
                Err(error)
            }
        }
    }

    fn record(&mut self, event: CdpEvent) {
        // A connection that closed since has nobody reading its events.
        let Some(buffer) = &mut self.buffer else {
            return;
        };
        // An event the buffer cannot hold is a visible gap.
        if let Err(error) = buffer.push(event)
            && let Err(gap) = buffer.mark_gap()
        {
            tracing::warn!("a tab's debugging events lost their order: {error}; {gap}");
        }
        self.recorded.send_replace(buffer.next());
    }

    fn page(&self, query: Query) -> Result<EventPage> {
        let buffer = self
            .buffer
            .as_ref()
            .ok_or_else(|| BrowserError::Connection("tab debugging connection ended".into()))?;
        let Some(after) = &query.after else {
            return Ok(EventPage {
                result: CdpEventsResult {
                    events: Vec::new(),
                    cursor: buffer.cursor(buffer.next()),
                    has_more: false,
                    truncated: false,
                },
                wait: false,
            });
        };
        let position = buffer.position(after)?;
        let matches: Vec<&CdpEvent> = buffer
            .entries()
            .filter(|entry| {
                entry.sequence >= position
                    && entry.target == query.target
                    && query
                        .methods
                        .as_ref()
                        .is_none_or(|methods| methods.contains(&entry.method))
            })
            .collect();
        let more = matches.len() > query.limit;
        let rows: Vec<CdpEvent> = matches.into_iter().take(query.limit).cloned().collect();
        let cursor = if more {
            rows.last().expect("nonempty limited page").sequence + 1
        } else {
            buffer.next()
        };
        let wait = rows.is_empty();
        Ok(EventPage {
            result: CdpEventsResult {
                events: rows,
                cursor: buffer.cursor(cursor),
                has_more: more,
                truncated: buffer.truncated_since(position),
            },
            wait,
        })
    }

    /// Closes every connection and joins their cleanup.
    async fn close_all(&mut self) -> Result<()> {
        let connections = std::mem::take(&mut self.connections);
        self.buffer = None;
        self.publish_callers();
        let mut result = Ok(());
        for (_, connection) in connections {
            result = after_cleanup(result, connection.close().await);
        }
        result
    }

    fn publish_callers(&self) {
        let mut callers: Vec<_> = self
            .connections
            .iter()
            .filter(|(_, connection)| connection.ended().is_none())
            .map(|(caller, _)| *caller)
            .collect();
        callers.sort();
        self.callers.send_replace(callers);
    }
}

/// One caller's debugging connection to a tab.
struct DebugConnection {
    handle: DebugHandle,
    stop: CancellationToken,
    /// Why the pump ended, once it has; its sender is the pump's, so a pump
    /// that ends without saying, aborted with its tab, closes the channel.
    ended: watch::Receiver<Option<String>>,
    task: AbortOnDropHandle<Result<()>>,
}

#[derive(Clone)]
struct Target {
    session: SessionId,
    parent: Option<SessionId>,
}

impl DebugConnection {
    async fn start(
        browser: &BrowserHandle,
        tab: &TargetId,
        ended: &CancellationToken,
        tasks: &TaskTracker,
        recording: mpsc::Sender<Recorded>,
    ) -> Result<Self> {
        let address = browser.call()?.websocket_address().clone();
        let mut socket = Connection::<WireEvent>::connect(address).await?;
        let attached = roundtrip(
            &mut socket,
            "Target.attachToTarget",
            json!({"targetId": tab, "flatten": true}),
            None,
        )
        .await?;
        let attached: AttachToTargetReturns =
            serde_json::from_value(attached).map_err(|error| {
                BrowserError::Cdp(chromiumoxide::error::CdpError::msg(error.to_string()))
            })?;
        let targets = HashMap::from([(
            "main".into(),
            Target {
                session: attached.session_id.clone(),
                parent: None,
            },
        )]);
        attach_descendants(&mut socket, attached.session_id)?;
        let (requests, receiver) = mpsc::channel(REQUESTS);
        let stop = ended.child_token();
        let (reason, ended) = watch::channel(None);
        let task = AbortOnDropHandle::new(tasks.spawn(pump(
            socket,
            receiver,
            targets,
            recording,
            stop.clone(),
            reason,
        )));
        Ok(Self {
            handle: DebugHandle { requests },
            stop,
            ended,
            task,
        })
    }

    /// Why the connection ended, or `None` while it lives.
    fn ended(&self) -> Option<String> {
        if let Some(reason) = &*self.ended.borrow() {
            return Some(reason.clone());
        }
        self.ended
            .has_changed()
            .is_err()
            .then(|| "its task stopped".into())
    }

    async fn close(self) -> Result<()> {
        self.stop.cancel();
        match tokio::time::timeout(CONTROL_TIMEOUT, self.task).await {
            Ok(result) => match result? {
                // The actor failed pending calls and dropped its owned socket.
                // Its transport loss is not a failure to release debug state.
                Err(BrowserError::Closed | BrowserError::Connection(_)) => Ok(()),
                other => other,
            },
            Err(_) => Err(BrowserError::Timeout),
        }
    }
}

async fn pump(
    mut socket: Connection<WireEvent>,
    mut requests: mpsc::Receiver<ConnectionRequest>,
    mut targets: HashMap<String, Target>,
    recording: mpsc::Sender<Recorded>,
    stop: CancellationToken,
    reason: watch::Sender<Option<String>>,
) -> Result<()> {
    let mut pending: HashMap<CallId, (String, oneshot::Sender<Result<Value>>)> = HashMap::new();
    let work = async {
        loop {
            tokio::select! {
                biased;
                _ = stop.cancelled() => return Ok(()),
                request = requests.recv() => match request {
                    None => return Ok(()),
                    Some(ConnectionRequest::Targets { reply }) => {
                        // A command that left needs no answer.
                        let _left = reply.send(targets.keys().cloned().collect());
                    }
                    Some(ConnectionRequest::Send { method, params, target, response }) => {
                        let Some(target) = targets.get(&target).cloned() else {
                            // A dropped invocation no longer needs its response.
                            let _left = response.send(Err(BrowserError::TargetNotFound));
                            continue;
                        };
                        let id = socket.submit_command(
                            method.clone().into(), Some(target.session), params,
                        ).map_err(|error| BrowserError::Cdp(chromiumoxide::error::CdpError::msg(error.to_string())))?;
                        pending.insert(id, (method, response));
                    }
                },
                message = socket.next() => {
                    match message.ok_or(BrowserError::Closed)?? {
                        Message::Response(response) => {
                            if let Some((method, sender)) = pending.remove(&response.id) {
                                let result = reply(response).and_then(|value| {
                                    catalog()?.validate(&method, "returns", &value)?;
                                    Ok(value)
                                });
                                // Invocation cancellation can drop this receiver; the
                                // connection still owns and cleans the debug effect.
                                let _left = sender.send(result);
                            } else if let Some(error) = response.error {
                                return Err(BrowserError::Cdp(chromiumoxide::error::CdpError::msg(error.to_string())));
                            }
                        }
                        Message::Event(event) => {
                            catalog()?.validate(&event.method, "event", &event.params)?;
                            if event.method == "Target.attachedToTarget" {
                                let attached: EventAttachedToTarget = serde_json::from_value(event.params.clone())
                                    .map_err(|error| BrowserError::Cdp(chromiumoxide::error::CdpError::msg(error.to_string())))?;
                                let id = attached.target_info.target_id.as_ref().to_owned();
                                targets.insert(id, Target {
                                    session: attached.session_id.clone(),
                                    parent: event.session_id.clone().map(SessionId::new),
                                });
                                attach_descendants(&mut socket, attached.session_id)?;
                            } else if event.method == "Target.detachedFromTarget" {
                                let detached: EventDetachedFromTarget = serde_json::from_value(event.params.clone())
                                    .map_err(|error| BrowserError::Cdp(chromiumoxide::error::CdpError::msg(error.to_string())))?;
                                targets.retain(|_, target| target.session != detached.session_id);
                            }
                            let target = targets.iter()
                                .find(|(_, target)| event.session_id.as_deref() == Some(target.session.as_ref()))
                                .map(|(id, _)| id.clone());
                            if let Some(target) = target {
                                let recorded = Recorded::Event(CdpEvent {
                                    sequence: 0,
                                    method: event.method,
                                    params: event.params,
                                    target,
                                });
                                // A stop while the owner is full ends the pump.
                                tokio::select! {
                                    biased;
                                    _ = stop.cancelled() => return Ok(()),
                                    sent = recording.send(recorded) => sent.map_err(|_| BrowserError::Closed)?,
                                }
                            }
                        }
                    }
                }
            }
        }
    }.await;
    requests.close();
    let failure = work
        .as_ref()
        .err()
        .map_or_else(|| "it was closed".into(), ToString::to_string);
    for (_, (_, sender)) in pending {
        // A cancelled invocation can already have dropped its response receiver.
        let _left = sender.send(Err(ended(&failure)));
    }
    while let Some(request) = requests.recv().await {
        match request {
            // A cancelled invocation can already have dropped its response receiver.
            ConnectionRequest::Send { response, .. } => {
                let _left = response.send(Err(ended(&failure)));
            }
            ConnectionRequest::Targets { reply } => {
                let _left = reply.send(Vec::new());
            }
        }
    }
    // Chrome scopes breakpoints, debug pauses and interception to the attached
    // session. Detach children before their parent and await acknowledgement;
    // dropping this dedicated connection then closes all remaining ownership.
    let cleanup = async {
        let mut sessions: Vec<_> = targets
            .iter()
            .map(|(id, target)| (id == "main", target.session.clone(), target.parent.clone()))
            .collect();
        sessions.sort_by_key(|(main, _, _)| *main);
        let mut result = Ok(());
        for (_, session, parent) in sessions {
            let detached = roundtrip(
                &mut socket,
                "Target.detachFromTarget",
                json!({"sessionId": session}),
                parent,
            )
            .await
            .map(|_| ());
            let detached = match detached {
                // No detach acknowledgement is possible after transport loss.
                // Dropping this private socket releases all of its sessions.
                Err(BrowserError::Closed | BrowserError::Connection(_)) => Ok(()),
                // Tab retirement may have detached this session before the actor
                // receives its target event. An absent session is already released.
                Err(BrowserError::Cdp(chromiumoxide::error::CdpError::Chrome(error)))
                    if error.code == -32602 && error.message == "No session with given id" =>
                {
                    Ok(())
                }
                other => other,
            };
            result = after_cleanup(result, detached);
        }
        result
    };
    let cleanup = tokio::time::timeout(CONTROL_TIMEOUT, cleanup)
        .await
        .map_err(|_| BrowserError::Timeout)
        .and_then(std::convert::identity);
    drop(socket);
    // The owner learns that this connection ended, unless it is closing it:
    // then it waits for this task and reads nothing meanwhile.
    reason.send_replace(Some(failure));
    tokio::select! {
        biased;
        _ = stop.cancelled() => {}
        // The owner ended with its tab.
        _closed = recording.send(Recorded::Ended) => {}
    }
    after_cleanup(work, cleanup)
}

/// The failure of a call on a debugging connection that ended, and why
/// (`browser.md` § CDP commands and events).
fn ended(reason: &str) -> BrowserError {
    BrowserError::Connection(format!("tab debugging connection ended: {reason}"))
}

/// Subscribe a CDP debug session only to iframe and worker descendants of its tab.
fn attach_descendants(socket: &mut Connection<WireEvent>, session: SessionId) -> Result<()> {
    let method = "Target.setAutoAttach";
    let params = json!({
        "autoAttach":true,
        "waitForDebuggerOnStart":false,
        "flatten":true,
        "filter":[
            {"type":"iframe"},
            {"type":"worker"},
            {"type":"shared_worker"},
            {"type":"service_worker"},
            {"exclude":true}
        ]
    });
    catalog()?.validate(method, "params", &params)?;
    socket
        .submit_command(method.into(), Some(session), params)
        .map_err(|error| {
            BrowserError::Cdp(chromiumoxide::error::CdpError::msg(error.to_string()))
        })?;
    Ok(())
}

/// Complete a driver-owned debugging setup/cleanup request on its private socket.
async fn roundtrip(
    socket: &mut Connection<WireEvent>,
    method: &str,
    params: Value,
    session: Option<SessionId>,
) -> Result<Value> {
    catalog()?.validate(method, "params", &params)?;
    let id = socket
        .submit_command(method.to_owned().into(), session, params)
        .map_err(|error| {
            BrowserError::Cdp(chromiumoxide::error::CdpError::msg(error.to_string()))
        })?;
    while let Some(message) = socket.next().await {
        if let Message::Response(response) = message?
            && response.id == id
        {
            let value = reply(response)?;
            catalog()?.validate(method, "returns", &value)?;
            return Ok(value);
        }
        // Setup precedes subscriptions; cleanup is terminal, so intervening events
        // cannot be exposed after a debugging generation has ended.
    }
    Err(BrowserError::Closed)
}

fn reply(response: chromiumoxide::types::Response) -> Result<Value> {
    match (response.result, response.error) {
        (Some(value), None) => Ok(value),
        (None, Some(error)) => Err(chromiumoxide::error::CdpError::Chrome(error).into()),
        _ => Err(BrowserError::Cdp(chromiumoxide::error::CdpError::msg(
            "malformed CDP response envelope",
        ))),
    }
}

// Chromiumoxide's generic JSON event wrapper does not map top-level sessionId.
// This envelope preserves it; payload validation still uses the pinned catalog.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WireEvent {
    method: String,
    params: Value,
    session_id: Option<String>,
}
impl Method for WireEvent {
    fn identifier(&self) -> MethodId {
        self.method.clone().into()
    }
}
impl EventMessage for WireEvent {
    fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use chromiumoxide::conn::MAX_CDP_MESSAGE_BYTES;
    use demi_command_package_browser_protocol::browser::BrowserErrorCode;
    use futures_util::SinkExt;
    use tokio_tungstenite::tungstenite::Message as WsMessage;

    use super::*;

    /// The id of the next command the peer's `socket` receives.
    async fn command_id(
        socket: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    ) -> u64 {
        let message = socket.next().await.unwrap().unwrap();
        let command: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
        command["id"].as_u64().unwrap()
    }

    /// About 0.1 s: two messages just over the 64 MiB limit cross the
    /// loopback.
    ///
    /// Planted defect this catches: a message over the limit rejected from
    /// its frame header, which leaves its payload on the socket and loses
    /// the connection, and with it the browser.
    #[tokio::test]
    async fn a_response_over_the_limit_fails_only_its_request_and_an_event_over_it_ends_the_connection()
     {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let peer = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let id = command_id(&mut socket).await;
            let data = "x".repeat(MAX_CDP_MESSAGE_BYTES);
            let oversized = format!(r#"{{"id":{id},"result":{{"data":"{data}"}}}}"#);
            socket.send(WsMessage::text(oversized)).await.unwrap();
            let id = command_id(&mut socket).await;
            let version = json!({"id": id, "result": {
                "protocolVersion": "1.3",
                "product": "Chrome/153.0.8010.36",
                "revision": "@0",
                "userAgent": "Mozilla/5.0",
                "jsVersion": "15.3",
            }});
            socket
                .send(WsMessage::text(version.to_string()))
                .await
                .unwrap();
            let event = format!(r#"{{"method":"Page.frameNavigated","params":{{"data":"{data}"}}}}"#);
            // The client may close before the whole event is written.
            let _ = socket.send(WsMessage::text(event)).await;
        });
        let mut connection = Connection::<WireEvent>::connect(format!("ws://{address}"))
            .await
            .unwrap();

        // The oversized response fails its request with a message naming
        // its size and the limit.
        let failure = roundtrip(&mut connection, "Browser.getVersion", json!({}), None)
            .await
            .unwrap_err();
        assert_eq!(failure.code(), BrowserErrorCode::ResultTooLarge, "{failure}");
        let size = MAX_CDP_MESSAGE_BYTES + r#"{"id":0,"result":{"data":""}}"#.len();
        assert_eq!(
            failure.to_string(),
            format!(
                "Chrome's answer is too large: the CDP message is {size} bytes, more than the {MAX_CDP_MESSAGE_BYTES} bytes a message may have"
            )
        );

        // The connection goes on: the next request is answered.
        let version = roundtrip(&mut connection, "Browser.getVersion", json!({}), None)
            .await
            .unwrap();
        assert_eq!(version["product"], "Chrome/153.0.8010.36");

        // An event answers no request, so losing it would leave its
        // listeners out of date: the connection ends with the reason.
        let ended = connection.next().await.unwrap().unwrap_err();
        assert!(
            ended.to_string().contains("and it answers no request"),
            "{ended}"
        );
        drop(connection);
        peer.await.unwrap();
    }
}

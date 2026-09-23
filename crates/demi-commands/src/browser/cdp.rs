//! Tab-scoped debugging on owned connections with pinned protocol validation.
//!
//! One task per tab owns its debugging connections, one per calling agent
//! node, and the events they record (`browser.md` § CDP commands and
//! events). Commands ask it for their connection and for pages of the
//! events; each connection's pump owns its socket and the targets it
//! attached, and answers the commands that use it.

use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
};

use chromiumoxide::{
    cdp::browser_protocol::target::{
        AttachToTargetReturns, EventAttachedToTarget, EventDetachedFromTarget,
        GetTargetInfoReturns, SessionId, TargetId,
    },
    conn::Connection,
    types::{CallId, EventMessage, Message, Method, MethodId},
};
use demi_command_service::InvocationContext;
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot, watch};
use tokio_util::{
    sync::CancellationToken,
    task::{AbortOnDropHandle, TaskTracker},
};

use super::{
    BrowserEnvironment, BrowserError, BrowserTab, Result,
    environment::BrowserHandle,
    history::Buffer,
    operation::{CONTROL_TIMEOUT, Operation, after_cleanup},
    protocol::{
        BrowserErrorCode, BrowserOperation, CDP_BYTES, CDP_EVENTS, Capability, CdpDetachResult,
        CdpEvent, CdpEventsResult, CdpSendResult, CdpTarget, CdpTargetsResult, DEFAULT_NODES,
    },
};

const DENIED_DOMAINS: &[&str] = &[
    "Target",
    "Browser",
    "SystemInfo",
    "Tethering",
    "HeadlessExperimental",
];
const DENIED_METHODS: &[&str] = &["Page.close", "Page.crash", "Page.setDownloadBehavior"];

/// Requests waiting for a tab's debugging owner or for a connection's pump;
/// a full queue holds back their senders.
const REQUESTS: usize = 16;
/// Events the pumps have read and the owner has not recorded yet; a full
/// queue holds back the pumps' reads.
const RECORDING: usize = 64;

/// The way to a tab's debugging owner.
pub(super) struct DebugSessions {
    requests: mpsc::Sender<SessionsRequest>,
    /// The callers with a connection open, for a timeout's diagnostics.
    callers: watch::Receiver<Vec<String>>,
    /// Where the next recorded event goes, so a waiting reader learns of new
    /// events.
    recorded: watch::Receiver<u64>,
    /// Cancelled once the owner has closed every connection and ended.
    finished: CancellationToken,
}

enum SessionsRequest {
    /// The caller's connection, made if it has none.
    Connect {
        caller: String,
        reply: oneshot::Sender<Result<DebugHandle>>,
    },
    /// Closes the caller's connection.
    Detach {
        caller: String,
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
struct Query {
    after: Option<String>,
    limit: usize,
    methods: Option<Vec<String>>,
    target: String,
}

/// A page of recorded events. `wait` when it holds none after a cursor: a
/// read with a timeout then waits for the next event.
struct EventPage {
    result: CdpEventsResult,
    wait: bool,
}

impl DebugSessions {
    /// Starts the debugging owner of the tab `target`; it ends with the tab.
    pub fn start(
        browser: BrowserHandle,
        target: TargetId,
        ended: CancellationToken,
        tasks: &TaskTracker,
    ) -> Self {
        let (requests, received) = mpsc::channel(REQUESTS);
        let (recording, recordings) = mpsc::channel(RECORDING);
        let (callers_sender, callers) = watch::channel(Vec::new());
        let (recorded_sender, recorded) = watch::channel(0);
        let finished = CancellationToken::new();
        let owner = SessionsOwner {
            browser,
            target,
            ended,
            tasks: tasks.clone(),
            connections: HashMap::new(),
            buffer: None,
            recording,
            callers: callers_sender,
            recorded: recorded_sender,
        };
        tasks.spawn(owner.run(received, recordings, finished.clone()));
        Self {
            requests,
            callers,
            recorded,
            finished,
        }
    }

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

    /// `caller`'s connection to the tab, made if it has none.
    async fn connect(&self, caller: &str) -> Result<DebugHandle> {
        let caller = caller.to_owned();
        self.ask(|reply| SessionsRequest::Connect { caller, reply })
            .await
    }

    /// Closes `caller`'s connection and joins its cleanup.
    pub async fn detach(&self, caller: &str) -> Result<()> {
        let caller = caller.to_owned();
        self.ask(|reply| SessionsRequest::Detach { caller, reply })
            .await
    }

    async fn events(&self, query: Query) -> Result<EventPage> {
        self.ask(|reply| SessionsRequest::Events { query, reply })
            .await
    }

    /// Watches the recorded events from now on.
    fn recorded(&self) -> watch::Receiver<u64> {
        let mut recorded = self.recorded.clone();
        recorded.mark_unchanged();
        recorded
    }

    /// The other callers whose connections to the tab are open.
    pub fn other_callers(&self, caller: Option<&str>) -> Vec<String> {
        self.callers
            .borrow()
            .iter()
            .filter(|owner| Some(owner.as_str()) != caller)
            .cloned()
            .collect()
    }

    /// Closes every connection; once this returns, all are gone.
    pub async fn release(&self) -> Result<()> {
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

struct SessionsOwner {
    browser: BrowserHandle,
    target: TargetId,
    /// The tab's end.
    ended: CancellationToken,
    tasks: TaskTracker,
    connections: HashMap<String, DebugConnection>,
    /// The tab's recorded events, while a connection is open.
    buffer: Option<Buffer<CdpEvent>>,
    recording: mpsc::Sender<Recorded>,
    callers: watch::Sender<Vec<String>>,
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

    async fn connect(&mut self, caller: String) -> Result<DebugHandle> {
        if let Some(connection) = self.connections.get(&caller) {
            if connection.ended.is_cancelled() {
                return Err(BrowserError::Connection(
                    "tab debugging connection ended".into(),
                ));
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
            .filter(|(_, connection)| !connection.ended.is_cancelled())
            .map(|(caller, _)| caller.clone())
            .collect();
        callers.sort();
        self.callers.send_replace(callers);
    }
}

/// Expose the design's denied list without copying the pinned method catalog.
pub(super) fn capability() -> Capability {
    Capability {
        id: "cdp".into(),
        available: true,
        reason: None,
        schema: Some(
            json!({"deniedDomains": DENIED_DOMAINS, "deniedMethods": DENIED_METHODS, "help": "demi browser cdp --help"}),
        ),
    }
}

/// Execute only pinned tab/child methods; never accept a caller session identifier.
pub(super) async fn execute(
    context: &InvocationContext,
    _environment: &BrowserEnvironment,
    tab: Option<&BrowserTab>,
    command: &BrowserOperation,
    cancel: &CancellationToken,
    deadline: tokio::time::Instant,
) -> Result<Value> {
    let tab = tab.ok_or(BrowserError::TabNotFound)?;
    let operation = Operation::for_tab(tab, cancel, deadline);
    let _session = tab.state.gate.try_checkout().ok_or(BrowserError::Busy)?;
    let owner = super::conversations::agent(context)?;
    let debug = &tab.state.debug;
    if matches!(command, BrowserOperation::CdpDetach(_)) {
        debug.detach(owner).await?;
        return super::output::value(CdpDetachResult {
            detached: tab.id().clone(),
        });
    }
    let parameters = if let BrowserOperation::CdpSend(input) = command {
        admit(&input.method)?;
        let params: Value = serde_json::from_str(&input.params)
            .map_err(|error| BrowserError::Configuration(error.to_string()))?;
        catalog()?
            .validate(&input.method, "params", &params)
            .map_err(|error| BrowserError::Configuration(error.to_string()))?;
        Some(params)
    } else {
        None
    };
    let connection = operation.run(debug.connect(owner)).await?;
    match command {
        BrowserOperation::CdpTargets(input) => {
            // A round trip flushes attachment events already queued by Chrome.
            operation
                .run(connection.send("Runtime.getIsolateId", json!({}), "main"))
                .await?;
            let targets = operation.run(connection.targets()).await?;
            let mut rows = Vec::with_capacity(targets.len());
            for id in targets {
                let value = match operation
                    .run(connection.send("Target.getTargetInfo", json!({}), &id))
                    .await
                {
                    Ok(value) => value,
                    // A child may close after the target snapshot was taken.
                    Err(BrowserError::TargetNotFound) => continue,
                    Err(error) => return Err(error),
                };
                let target: GetTargetInfoReturns =
                    serde_json::from_value(value).map_err(|error| {
                        BrowserError::Cdp(chromiumoxide::error::CdpError::msg(error.to_string()))
                    })?;
                rows.push(CdpTarget {
                    id,
                    kind: target.target_info.r#type,
                    url: target.target_info.url,
                });
            }
            rows.sort_by(|left, right| left.id.cmp(&right.id));
            let offset = input.offset.unwrap_or(0);
            let limit = input.limit.unwrap_or(DEFAULT_NODES);
            let count = rows.len();
            let targets = rows.into_iter().skip(offset).take(limit).collect();
            super::output::value(CdpTargetsResult {
                targets,
                truncated: offset.saturating_add(limit) < count,
            })
        }
        BrowserOperation::CdpSend(input) => {
            let params = parameters.expect("send parameters were validated before attachment");
            let result = operation
                .run(connection.send(
                    &input.method,
                    params,
                    input.target.as_deref().unwrap_or("main"),
                ))
                .await;
            let sent = |result| CdpSendResult {
                method: input.method.clone(),
                result,
            };
            if result.as_ref().is_err_and(|error| {
                matches!(
                    error.code(),
                    BrowserErrorCode::Cancelled | BrowserErrorCode::Timeout
                )
            }) {
                let cleanup = debug.detach(owner).await;
                return after_cleanup(result.map(sent), cleanup).and_then(super::output::value);
            }
            super::output::value(sent(result?))
        }
        BrowserOperation::CdpEvents(input) => {
            if let Some(methods) = &input.method {
                for method in methods {
                    catalog()?.schema(method, "event")?;
                }
            }
            let target = input.target.as_deref().unwrap_or("main");
            if !operation
                .run(connection.targets())
                .await?
                .iter()
                .any(|known| known == target)
            {
                return Err(BrowserError::TargetNotFound);
            }
            loop {
                // Watched before the read, so an event recorded after it wakes the wait.
                let mut recorded = debug.recorded();
                let page = operation
                    .run(debug.events(Query {
                        after: input.after.clone(),
                        limit: input.limit.unwrap_or(DEFAULT_NODES),
                        methods: input.method.clone(),
                        target: target.to_owned(),
                    }))
                    .await?;
                if !page.wait || input.timeout.is_none() || tokio::time::Instant::now() >= deadline
                {
                    return super::output::value(page.result);
                }
                let waited = operation
                    .run(async { recorded.changed().await.map_err(|_| BrowserError::Closed) })
                    .await;
                match waited {
                    Ok(()) => {}
                    Err(error) if context.cancellation.is_cancelled() => {
                        return after_cleanup(Err(error), debug.detach(owner).await);
                    }
                    Err(BrowserError::Timeout) => return super::output::value(page.result),
                    Err(BrowserError::Cancelled) if tokio::time::Instant::now() >= deadline => {
                        return super::output::value(page.result);
                    }
                    Err(error) => return Err(error),
                }
            }
        }
        _ => unreachable!("CDP detach was handled before targets/send/events"),
    }
}

/// One caller's debugging connection to a tab.
struct DebugConnection {
    handle: DebugHandle,
    stop: CancellationToken,
    /// Cancelled by the pump as it ends, however it ends.
    ended: CancellationToken,
    task: AbortOnDropHandle<Result<()>>,
}

/// The way to a connection's pump.
#[derive(Clone)]
struct DebugHandle {
    requests: mpsc::Sender<Request>,
}

enum Request {
    Send {
        method: String,
        params: Value,
        target: String,
        response: oneshot::Sender<Result<Value>>,
    },
    /// The targets the connection attached.
    Targets { reply: oneshot::Sender<Vec<String>> },
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
        let pump_ended = CancellationToken::new();
        let task = AbortOnDropHandle::new(tasks.spawn(pump(
            socket,
            receiver,
            targets,
            recording,
            stop.clone(),
            pump_ended.clone(),
        )));
        Ok(Self {
            handle: DebugHandle { requests },
            stop,
            ended: pump_ended,
            task,
        })
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

impl DebugHandle {
    async fn send(&self, method: &str, params: Value, target: &str) -> Result<Value> {
        let (response, result) = oneshot::channel();
        self.requests
            .send(Request::Send {
                method: method.into(),
                params,
                target: target.into(),
                response,
            })
            .await
            .map_err(|_| BrowserError::Closed)?;
        result.await.map_err(|_| BrowserError::Closed)?
    }

    async fn targets(&self) -> Result<Vec<String>> {
        let (reply, targets) = oneshot::channel();
        self.requests
            .send(Request::Targets { reply })
            .await
            .map_err(|_| BrowserError::Closed)?;
        targets.await.map_err(|_| BrowserError::Closed)
    }
}

async fn pump(
    mut socket: Connection<WireEvent>,
    mut requests: mpsc::Receiver<Request>,
    mut targets: HashMap<String, Target>,
    recording: mpsc::Sender<Recorded>,
    stop: CancellationToken,
    ended: CancellationToken,
) -> Result<()> {
    let _ended = ended.clone().drop_guard();
    let mut pending: HashMap<CallId, (String, oneshot::Sender<Result<Value>>)> = HashMap::new();
    let work = async {
        loop {
            tokio::select! {
                biased;
                _ = stop.cancelled() => return Ok(()),
                request = requests.recv() => match request {
                    None => return Ok(()),
                    Some(Request::Targets { reply }) => {
                        // A command that left needs no answer.
                        let _left = reply.send(targets.keys().cloned().collect());
                    }
                    Some(Request::Send { method, params, target, response }) => {
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
        .map_or_else(|| "debugging connection closed".into(), ToString::to_string);
    for (_, (_, sender)) in pending {
        // A cancelled invocation can already have dropped its response receiver.
        let _left = sender.send(Err(BrowserError::Cdp(chromiumoxide::error::CdpError::msg(
            failure.clone(),
        ))));
    }
    while let Some(request) = requests.recv().await {
        match request {
            // A cancelled invocation can already have dropped its response receiver.
            Request::Send { response, .. } => {
                let _left = response.send(Err(BrowserError::Cdp(
                    chromiumoxide::error::CdpError::msg(failure.clone()),
                )));
            }
            Request::Targets { reply } => {
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
    ended.cancel();
    tokio::select! {
        biased;
        _ = stop.cancelled() => {}
        // The owner ended with its tab.
        _closed = recording.send(Recorded::Ended) => {}
    }
    after_cleanup(work, cleanup)
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
        (None, Some(error)) => Err(BrowserError::Cdp(chromiumoxide::error::CdpError::Chrome(
            error,
        ))),
        _ => Err(BrowserError::Cdp(chromiumoxide::error::CdpError::msg(
            "malformed CDP response envelope",
        ))),
    }
}

fn admit(method: &str) -> Result<()> {
    let (domain, _) = method
        .split_once('.')
        .ok_or_else(|| BrowserError::Configuration("CDP method must be Domain.method".into()))?;
    if DENIED_DOMAINS.contains(&domain) || DENIED_METHODS.contains(&method) {
        return Err(BrowserError::CdpMethodDenied(method.into()));
    }
    catalog()?.schema(method, "params")?;
    Ok(())
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

struct Catalog {
    definitions: Value,
    schemas: HashMap<String, Value>,
    /// The validators compiled so far. A std mutex: every debugging pump and
    /// command in the process shares this cache, and each holds the mutex
    /// only to look up or to add a validator, never while compiling one or
    /// across an await.
    compiled: std::sync::Mutex<HashMap<String, Arc<jsonschema::Validator>>>,
}

fn catalog() -> Result<&'static Catalog> {
    static CATALOG: OnceLock<std::result::Result<Catalog, String>> = OnceLock::new();
    CATALOG
        .get_or_init(Catalog::load)
        .as_ref()
        .map_err(|error| BrowserError::Cdp(chromiumoxide::error::CdpError::msg(error.clone())))
}

impl Catalog {
    fn load() -> std::result::Result<Self, String> {
        let protocol: PinnedProtocol = serde_json::from_str(include_str!(
            "../../../../vendor/chromiumoxide_cdp/pdl/protocol.json"
        ))
        .map_err(|error| error.to_string())?;
        let mut definitions = serde_json::Map::new();
        let mut schemas = HashMap::new();
        for domain in protocol.domains {
            for shape in domain.types.into_iter().flatten() {
                definitions.insert(
                    format!(
                        "{}.{}",
                        domain.domain,
                        shape.id.as_deref().ok_or("CDP type has no id")?
                    ),
                    shape.schema(&domain.domain)?,
                );
            }
            for command in domain.commands {
                schemas.insert(
                    format!("{}.{}:params", domain.domain, command.name),
                    object_schema(&command.parameters, &domain.domain)?,
                );
                schemas.insert(
                    format!("{}.{}:returns", domain.domain, command.name),
                    object_schema(&command.returns, &domain.domain)?,
                );
            }
            for event in domain.events {
                schemas.insert(
                    format!("{}.{}:event", domain.domain, event.name),
                    object_schema(&event.parameters, &domain.domain)?,
                );
            }
        }
        Ok(Self {
            definitions: Value::Object(definitions),
            schemas,
            compiled: std::sync::Mutex::new(HashMap::new()),
        })
    }
    fn cache(&self) -> std::sync::MutexGuard<'_, HashMap<String, Arc<jsonschema::Validator>>> {
        // Nothing that can panic runs while the mutex is held.
        self.compiled
            .lock()
            .expect("the CDP schema cache is intact")
    }

    fn schema(&self, method: &str, kind: &str) -> Result<&Value> {
        self.schemas
            .get(&format!("{method}:{kind}"))
            .ok_or_else(|| {
                BrowserError::Configuration(format!("unknown pinned CDP {kind} method: {method}"))
            })
    }
    fn validate(&self, method: &str, kind: &str, value: &Value) -> Result<()> {
        let key = format!("{method}:{kind}");
        let cached = self.cache().get(&key).cloned();
        let validator = match cached {
            Some(validator) => validator,
            None => {
                let mut schema = self
                    .schema(method, kind)
                    .map_err(|error| {
                        BrowserError::Cdp(chromiumoxide::error::CdpError::msg(error.to_string()))
                    })?
                    .clone();
                schema["$defs"] = self.definitions.clone();
                let validator = Arc::new(jsonschema::options().offline().build(&schema).map_err(
                    |error| {
                        BrowserError::Cdp(chromiumoxide::error::CdpError::msg(error.to_string()))
                    },
                )?);
                // Another caller may have compiled the same schema meanwhile;
                // either copy validates alike.
                self.cache().entry(key).or_insert(validator).clone()
            }
        };
        validator.validate(value).map_err(|error| {
            BrowserError::Cdp(chromiumoxide::error::CdpError::msg(format!(
                "invalid {method} {kind}: {error}"
            )))
        })
    }
}

#[derive(Deserialize)]
struct PinnedProtocol {
    domains: Vec<Domain>,
}
#[derive(Deserialize)]
struct Domain {
    domain: String,
    #[serde(default)]
    types: Option<Vec<Shape>>,
    #[serde(default)]
    commands: Vec<Declaration>,
    #[serde(default)]
    events: Vec<Declaration>,
}
#[derive(Deserialize)]
struct Declaration {
    name: String,
    #[serde(default)]
    parameters: Vec<Shape>,
    #[serde(default)]
    returns: Vec<Shape>,
}
#[derive(Deserialize)]
struct Shape {
    id: Option<String>,
    name: Option<String>,
    #[serde(default)]
    optional: bool,
    #[serde(rename = "$ref")]
    reference: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
    #[serde(rename = "enum")]
    variants: Option<Vec<String>>,
    items: Option<Box<Shape>>,
    properties: Option<Vec<Shape>>,
}
impl Shape {
    fn schema(&self, domain: &str) -> std::result::Result<Value, String> {
        if let Some(reference) = &self.reference {
            return Ok(
                json!({"$ref":format!("#/$defs/{}",if reference.contains('.') {reference.clone()}else{format!("{domain}.{reference}")})}),
            );
        }
        let kind = self
            .kind
            .as_deref()
            .ok_or("CDP type has no type or reference")?;
        let mut schema = match kind {
            "any" => json!({}),
            "binary" => {
                json!({"type":"string","pattern":"^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$"})
            }
            "array" => {
                json!({"type":"array","items":self.items.as_ref().ok_or("CDP array has no items")?.schema(domain)?})
            }
            "object" if self.properties.is_some() => {
                object_schema(self.properties.as_ref().expect("properties exist"), domain)?
            }
            "object" | "string" | "integer" | "number" | "boolean" => json!({"type":kind}),
            _ => return Err(format!("unknown pinned CDP type {kind}")),
        };
        if let Some(variants) = &self.variants {
            schema["enum"] = json!(variants);
        }
        Ok(schema)
    }
}

/// Translate pinned CDP record declarations into strict JSON Schema objects.
fn object_schema(properties: &[Shape], domain: &str) -> std::result::Result<Value, String> {
    let mut members = serde_json::Map::new();
    let mut required = Vec::new();
    for property in properties {
        let name = property.name.as_ref().ok_or("CDP parameter has no name")?;
        members.insert(name.clone(), property.schema(domain)?);
        if !property.optional {
            required.push(name);
        }
    }
    Ok(
        json!({"type":"object","properties":members,"required":required,"additionalProperties":false}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chromiumoxide::cdp::browser_protocol::target::GetTargetsParams;
    use demi_command_service::protocol::CommandLocale;
    use futures_util::FutureExt;
    use std::{path::PathBuf, time::Duration};

    /// Inject extension failure through a private CDP connection; the public
    /// tab-scoped debugger deliberately cannot address this worker.
    #[tokio::test]
    #[ignore = "requires pinned real Chrome for Testing"]
    async fn capture_extension_reload_preserves_pages_and_recreates_its_worker() {
        let executable = PathBuf::from(std::env::var_os("DEMI_TEST_CHROME").unwrap());
        let options = super::super::LaunchOptions::pinned(
            executable,
            CommandLocale {
                time_zone: "UTC".into(),
                languages: vec!["en-US".into()],
            },
        )
        .unwrap();
        let exercise = |environment: BrowserEnvironment| async move {
            let work = async move {
                let cancel = CancellationToken::new();
                let timeout = Duration::from_secs(30);
                let tab = environment.open("about:blank", &cancel, timeout).await?;
                let address = environment.browser.call()?.websocket_address().clone();
                let mut socket = Connection::<WireEvent>::connect(address).await?;
                let worker_url = format!(
                    "chrome-extension://{}/background.js",
                    super::super::launch::CAPTURE_EXTENSION_ID
                );
                let mut previous = None;
                for round in 0..4 {
                    let target = tokio::time::timeout(timeout, async {
                        loop {
                            let targets = environment
                                .browser
                                .call()?
                                .execute(GetTargetsParams::default())
                                .await?;
                            // Target discovery precedes worker initialization. The
                            // offscreen document proves its startup code has run.
                            if !targets.result.target_infos.iter().any(|target| {
                                target.url == worker_url.replace("background.js", "offscreen.html")
                            }) {
                                tokio::time::sleep(Duration::from_millis(50)).await;
                                continue;
                            }
                            if let Some(target) =
                                targets.result.target_infos.into_iter().find(|target| {
                                    target.url == worker_url
                                        && previous.as_ref() != Some(&target.target_id)
                                })
                            {
                                return Ok::<_, BrowserError>(target.target_id);
                            }
                            tokio::time::sleep(Duration::from_millis(50)).await;
                        }
                    })
                    .await
                    .expect("the capture worker returns after every reload")?;
                    assert_eq!(environment.tabs(&cancel, timeout).await?.len(), 1);
                    assert_eq!(
                        tab.read_only("document.URL", &cancel, timeout).await?,
                        json!("about:blank")
                    );
                    if round == 3 {
                        break;
                    }
                    let attached = roundtrip(
                        &mut socket,
                        "Target.attachToTarget",
                        json!({"targetId":target,"flatten":true}),
                        None,
                    )
                    .await?;
                    let attached: AttachToTargetReturns = serde_json::from_value(attached).unwrap();
                    let evaluated = roundtrip(
                        &mut socket,
                        "Runtime.evaluate",
                        json!({
                            "expression": "setTimeout(() => chrome.runtime.reload(), 100); true",
                            "returnByValue": true,
                        }),
                        Some(attached.session_id),
                    )
                    .await?;
                    assert_eq!(evaluated["result"]["value"], true, "{evaluated}");
                    previous = Some(target);
                }
                Ok::<(), BrowserError>(())
            };
            // Join Chrome retirement before reporting an assertion or deadline.
            Ok(
                std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(60), work))
                    .catch_unwind()
                    .await,
            )
        };
        let outcome = super::super::with_browser(options, CancellationToken::new(), exercise)
            .await
            .unwrap();
        match outcome {
            Ok(result) => result.expect("extension reload deadline").unwrap(),
            Err(panic) => std::panic::resume_unwind(panic),
        }
    }
}

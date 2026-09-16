//! Tab-scoped debugging on owned connections with pinned protocol validation.

use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
};

use chromiumoxide::{
    cdp::browser_protocol::target::{
        AttachToTargetReturns, EventAttachedToTarget, EventDetachedFromTarget,
        GetTargetInfoReturns, SessionId,
    },
    conn::Connection,
    types::{CallId, EventMessage, Message, Method, MethodId},
};
use demi_command_service::InvocationContext;
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::{Mutex, Notify, mpsc, oneshot};
use tokio_util::{sync::CancellationToken, task::AbortOnDropHandle};

use super::{
    BrowserEnvironment, BrowserError, BrowserTab, Result,
    history::Buffer,
    operation::{CONTROL_TIMEOUT, Operation},
    protocol::{BrowserCommand, CDP_BYTES, CDP_EVENTS, DEFAULT_NODES},
};

const DENIED_DOMAINS: &[&str] = &[
    "Target",
    "Browser",
    "SystemInfo",
    "Tethering",
    "HeadlessExperimental",
];
const DENIED_METHODS: &[&str] = &["Page.close", "Page.crash", "Page.setDownloadBehavior"];

#[derive(Default)]
pub(super) struct State {
    connections: HashMap<String, DebugConnection>,
    events: Option<Arc<Events>>,
}

impl State {
    /// Name other nodes whose debugging connections are still open on this tab.
    pub(super) fn other_callers(&self, caller: Option<&str>) -> Vec<String> {
        let mut callers: Vec<_> = self
            .connections
            .iter()
            .filter(|(owner, connection)| {
                Some(owner.as_str()) != caller && !connection.task.is_finished()
            })
            .map(|(owner, _)| owner.clone())
            .collect();
        callers.sort();
        callers
    }
}

struct DebugConnection {
    handle: DebugHandle,
    task: AbortOnDropHandle<Result<()>>,
}

#[derive(Clone)]
struct DebugHandle {
    requests: mpsc::Sender<Request>,
    stop: CancellationToken,
    targets: Arc<Mutex<HashMap<String, Target>>>,
}

struct Request {
    method: String,
    params: Value,
    target: String,
    response: oneshot::Sender<Result<Value>>,
}

#[derive(Clone)]
struct Target {
    session: SessionId,
    parent: Option<SessionId>,
}

struct Events {
    buffer: Mutex<Buffer>,
    changed: Notify,
}

impl Events {
    fn new() -> Result<Self> {
        Ok(Self {
            buffer: Mutex::new(Buffer::new("cdp", CDP_EVENTS, CDP_BYTES)?),
            changed: Notify::new(),
        })
    }

    async fn push(&self, event: WireEvent, target: &str) -> Result<()> {
        self.buffer.lock().await.push(json!({
            "method": event.method,
            "params": event.params,
            "target": target
        }))?;
        self.changed.notify_waiters();
        Ok(())
    }
}

/// Expose the design's denied list without copying the pinned method catalog.
pub(super) fn capability() -> Value {
    json!({"id":"cdp", "available":true, "schema":{"deniedDomains":DENIED_DOMAINS,"deniedMethods":DENIED_METHODS,"help":"demi browser cdp --help"}})
}

/// Execute only pinned tab/child methods; never accept a caller session identifier.
pub(super) async fn execute(
    context: &InvocationContext,
    environment: &BrowserEnvironment,
    tab: Option<&BrowserTab>,
    command: &BrowserCommand,
    cancel: &CancellationToken,
    deadline: tokio::time::Instant,
) -> Result<Value> {
    let tab = tab.ok_or(BrowserError::TabNotFound)?;
    let operation = Operation::until(&tab.ended, cancel, deadline);
    let _guard = tab
        .state
        .operations
        .try_lock()
        .map_err(|_| BrowserError::Busy)?;
    let owner = context
        .request
        .caller
        .as_deref()
        .ok_or_else(|| BrowserError::Configuration("CDP requires a trusted caller".into()))?;
    if matches!(command, BrowserCommand::CdpDetach(_)) {
        cancel_owner(tab, owner).await?;
        return Ok(json!({"detached":tab.id()}));
    }
    let mut state = tab.state.cdp.lock().await;
    let parameters = if let BrowserCommand::CdpSend(input) = command {
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
    if !state.connections.contains_key(owner) {
        let events = match &state.events {
            Some(events) => events.clone(),
            None => {
                let events = Arc::new(Events::new()?);
                state.events = Some(events.clone());
                events
            }
        };
        let connection = operation
            .run(DebugConnection::start(environment, tab, events))
            .await?;
        state.connections.insert(owner.into(), connection);
    }
    let connection = state.connections.get(owner).expect("debug owner exists");
    if connection.task.is_finished() {
        return Err(BrowserError::Connection(
            "tab debugging connection ended".into(),
        ));
    }
    let connection = connection.handle.clone();
    let events = state
        .events
        .as_ref()
        .expect("debug event buffer exists")
        .clone();
    drop(state);
    match command {
        BrowserCommand::CdpTargets(input) => {
            // A round trip flushes attachment events already queued by Chrome.
            operation
                .run(connection.send("Runtime.getIsolateId", json!({}), "main"))
                .await?;
            let targets: Vec<_> = connection.targets.lock().await.keys().cloned().collect();
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
                rows.push(
                    json!({"id":id,"kind":target.target_info.r#type,"url":target.target_info.url}),
                );
            }
            rows.sort_by(|left, right| left["id"].as_str().cmp(&right["id"].as_str()));
            let offset = usize::try_from(input.offset.unwrap_or(0))
                .map_err(|_| BrowserError::Configuration("offset is too large".into()))?;
            let limit = usize::try_from(
                input
                    .limit
                    .unwrap_or(u64::try_from(DEFAULT_NODES).expect("default fits")),
            )
            .map_err(|_| BrowserError::Configuration("limit is too large".into()))?;
            let count = rows.len();
            let rows: Vec<_> = rows.into_iter().skip(offset).take(limit).collect();
            Ok(json!({"targets":rows,"truncated":offset.saturating_add(limit)<count}))
        }
        BrowserCommand::CdpSend(input) => {
            let params = parameters.expect("send parameters were validated before attachment");
            let result = operation
                .run(connection.send(
                    &input.method,
                    params,
                    input.target.as_deref().unwrap_or("main"),
                ))
                .await;
            if result
                .as_ref()
                .is_err_and(|error| matches!(error.code(), "cancelled" | "timeout"))
            {
                let cleanup = cancel_owner(tab, owner).await;
                return super::operation::after_cleanup(
                    result.map(|result| json!({"method":input.method,"result":result})),
                    cleanup,
                );
            }
            Ok(json!({"method":input.method,"result":result?}))
        }
        BrowserCommand::CdpEvents(input) => {
            if let Some(methods) = &input.method {
                for method in methods {
                    catalog()?.schema(method, "event")?;
                }
            }
            let target = input.target.as_deref().unwrap_or("main");
            if !connection.targets.lock().await.contains_key(target) {
                return Err(BrowserError::TargetNotFound);
            }
            loop {
                let changed = events.changed.notified();
                let buffer = events.buffer.lock().await;
                let Some(after) = &input.after else {
                    return Ok(
                        json!({"events":[],"cursor":buffer.cursor(buffer.next()),"hasMore":false,"truncated":false}),
                    );
                };
                let position = buffer.position(after)?;
                let limit = usize::try_from(
                    input
                        .limit
                        .unwrap_or(u64::try_from(DEFAULT_NODES).expect("default fits")),
                )
                .map_err(|_| BrowserError::Configuration("limit is too large".into()))?;
                let matches: Vec<_> = buffer
                    .entries()
                    .filter(|entry| {
                        entry["sequence"]
                            .as_u64()
                            .is_some_and(|sequence| sequence >= position)
                            && entry["target"] == target
                            && input.method.as_ref().is_none_or(|methods| {
                                methods.iter().any(|method| entry["method"] == *method)
                            })
                    })
                    .collect();
                let more = matches.len() > limit;
                let rows: Vec<_> = matches.into_iter().take(limit).cloned().collect();
                let cursor = if more {
                    rows.last()
                        .and_then(|entry| entry["sequence"].as_u64())
                        .expect("nonempty limited page")
                        + 1
                } else {
                    buffer.next()
                };
                let result = json!({"events":rows,"cursor":buffer.cursor(cursor),"hasMore":more,"truncated":buffer.truncated_since(position)});
                if !rows.is_empty()
                    || input.timeout.is_none()
                    || tokio::time::Instant::now() >= deadline
                {
                    return Ok(result);
                }
                drop(buffer);
                match operation
                    .run(async {
                        changed.await;
                        Ok(())
                    })
                    .await
                {
                    Ok(()) => {}
                    Err(error) if context.cancellation.is_cancelled() => {
                        return super::operation::after_cleanup(
                            Err(error),
                            cancel_owner(tab, owner).await,
                        );
                    }
                    Err(BrowserError::Timeout) => return Ok(result),
                    Err(BrowserError::Cancelled) if tokio::time::Instant::now() >= deadline => {
                        return Ok(result);
                    }
                    Err(error) => return Err(error),
                }
            }
        }
        _ => unreachable!("CDP detach was handled before targets/send/events"),
    }
}

/// Cancel only this caller's owned debugging sessions and join their cleanup.
pub(super) async fn cancel_owner(tab: &BrowserTab, caller: &str) -> Result<()> {
    let mut state = tab.state.cdp.lock().await;
    let connection = state.connections.remove(caller);
    if state.connections.is_empty() {
        state.events = None;
    }
    drop(state);
    if let Some(connection) = connection {
        connection.close().await?;
    }
    Ok(())
}

/// Release all debugging resources before acknowledging tab retirement.
pub(super) async fn release(tab: &BrowserTab) -> Result<()> {
    let mut state = tab.state.cdp.lock().await;
    let connections = std::mem::take(&mut state.connections);
    state.events = None;
    drop(state);
    let mut result = Ok(());
    for (_, connection) in connections {
        result = super::operation::after_cleanup(result, connection.close().await);
    }
    result
}

impl DebugConnection {
    async fn start(
        environment: &BrowserEnvironment,
        tab: &BrowserTab,
        events: Arc<Events>,
    ) -> Result<Self> {
        let browser = environment.browser.upgrade().ok_or(BrowserError::Closed)?;
        let address = browser.lock().await.websocket_address().clone();
        let mut socket = Connection::<WireEvent>::connect(address).await?;
        let attached = roundtrip(
            &mut socket,
            "Target.attachToTarget",
            json!({"targetId":tab.target_id(),"flatten":true}),
            None,
        )
        .await?;
        let attached: AttachToTargetReturns =
            serde_json::from_value(attached).map_err(|error| {
                BrowserError::Cdp(chromiumoxide::error::CdpError::msg(error.to_string()))
            })?;
        let targets = Arc::new(Mutex::new(HashMap::from([(
            "main".into(),
            Target {
                session: attached.session_id.clone(),
                parent: None,
            },
        )])));
        attach_descendants(&mut socket, attached.session_id)?;
        let (requests, receiver) = mpsc::channel(16);
        let stop = tab.ended.child_token();
        let task = AbortOnDropHandle::new(tokio::spawn(pump(
            socket,
            receiver,
            targets.clone(),
            events,
            stop.clone(),
        )));
        Ok(Self {
            handle: DebugHandle {
                requests,
                stop,
                targets,
            },
            task,
        })
    }

    async fn close(self) -> Result<()> {
        self.handle.stop.cancel();
        match tokio::time::timeout(CONTROL_TIMEOUT, self.task).await {
            Ok(result) => result?,
            Err(_) => Err(BrowserError::Timeout),
        }
    }
}

impl DebugHandle {
    async fn send(&self, method: &str, params: Value, target: &str) -> Result<Value> {
        let (response, result) = oneshot::channel();
        self.requests
            .send(Request {
                method: method.into(),
                params,
                target: target.into(),
                response,
            })
            .await
            .map_err(|_| BrowserError::Closed)?;
        result.await.map_err(|_| BrowserError::Closed)?
    }
}

async fn pump(
    mut socket: Connection<WireEvent>,
    mut requests: mpsc::Receiver<Request>,
    targets: Arc<Mutex<HashMap<String, Target>>>,
    events: Arc<Events>,
    stop: CancellationToken,
) -> Result<()> {
    let mut pending: HashMap<CallId, (String, oneshot::Sender<Result<Value>>)> = HashMap::new();
    let work = async {
        loop {
            tokio::select! {
                biased;
                _ = stop.cancelled() => return Ok(()),
                request = requests.recv() => {
                    let Some(request) = request else {
                        return Ok(());
                    };
                    let target = targets.lock().await.get(&request.target).cloned();
                    let Some(target) = target else {
                        // A dropped invocation no longer needs its response.
                        let _ = request.response.send(Err(BrowserError::TargetNotFound));
                        continue;
                    };
                    let id = socket.submit_command(
                        request.method.clone().into(), Some(target.session), request.params,
                    ).map_err(|error| BrowserError::Cdp(chromiumoxide::error::CdpError::msg(error.to_string())))?;
                    pending.insert(id, (request.method, request.response));
                }
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
                                let _ = sender.send(result);
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
                                targets.lock().await.insert(id, Target {
                                    session: attached.session_id.clone(),
                                    parent: event.session_id.clone().map(SessionId::new),
                                });
                                attach_descendants(&mut socket, attached.session_id)?;
                            } else if event.method == "Target.detachedFromTarget" {
                                let detached: EventDetachedFromTarget = serde_json::from_value(event.params.clone())
                                    .map_err(|error| BrowserError::Cdp(chromiumoxide::error::CdpError::msg(error.to_string())))?;
                                targets.lock().await.retain(|_, target| target.session != detached.session_id);
                            }
                            let target = targets.lock().await.iter()
                                .find(|(_, target)| event.session_id.as_deref() == Some(target.session.as_ref()))
                                .map(|(id, _)| id.clone());
                            if let Some(target) = target {
                                events.push(event, &target).await?;
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
        let _ = sender.send(Err(BrowserError::Cdp(chromiumoxide::error::CdpError::msg(
            failure.clone(),
        ))));
    }
    while let Some(request) = requests.recv().await {
        // A cancelled invocation can already have dropped its response receiver.
        let _ = request
            .response
            .send(Err(BrowserError::Cdp(chromiumoxide::error::CdpError::msg(
                failure.clone(),
            ))));
    }
    // Chrome scopes breakpoints, debug pauses and interception to the attached
    // session. Detach children before their parent and await acknowledgement;
    // dropping this dedicated connection then closes all remaining ownership.
    let cleanup = async {
        let mut sessions: Vec<_> = targets
            .lock()
            .await
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
                // Tab retirement may have detached this session before the actor
                // receives its target event. An absent session is already released.
                Err(BrowserError::Cdp(chromiumoxide::error::CdpError::Chrome(error)))
                    if error.code == -32602 && error.message == "No session with given id" =>
                {
                    Ok(())
                }
                other => other,
            };
            result = super::operation::after_cleanup(result, detached);
        }
        result
    };
    let cleanup = tokio::time::timeout(CONTROL_TIMEOUT, cleanup)
        .await
        .map_err(|_| BrowserError::Timeout)
        .and_then(std::convert::identity);
    drop(socket);
    super::operation::after_cleanup(work, cleanup)
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
    fn schema(&self, method: &str, kind: &str) -> Result<&Value> {
        self.schemas
            .get(&format!("{method}:{kind}"))
            .ok_or_else(|| {
                BrowserError::Configuration(format!("unknown pinned CDP {kind} method: {method}"))
            })
    }
    fn validate(&self, method: &str, kind: &str, value: &Value) -> Result<()> {
        let key = format!("{method}:{kind}");
        let mut compiled = self.compiled.lock().map_err(|_| {
            BrowserError::Cdp(chromiumoxide::error::CdpError::msg(
                "CDP schema cache poisoned",
            ))
        })?;
        let validator = match compiled.get(&key) {
            Some(validator) => validator.clone(),
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
                compiled.insert(key, validator.clone());
                validator
            }
        };
        drop(compiled);
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

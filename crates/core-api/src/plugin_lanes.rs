//! The lanes of a program's plugins in one core: the program opens one for
//! each of its plugins, `{ kind: "plugin", plugin: "<id>" }`, on its
//! connection to the core, and the core's requests of the plugin travel on
//! it in the messages of `crates/plugin-interface/src/wire.rs`, from the
//! plugin's instance on the core's thread (`crates/core-api/src/process/plugins.rs`).
//!
//! A message that holds bytes is a binary lane message: the JSON, each value
//! of bytes in it the index of a buffer, then the buffers
//! (`crates/shared-types/src/bytes.rs`):
//!
//! ```text
//! u32 JSON length | JSON | u32 length | bytes | u32 length | bytes ...
//! ```
//!
//! One lane of a plugin is open at a time. The lane's end ends the requests
//! it carried.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use axum::body::Bytes;
use axum::extract::ws::close_code;
use demi_core::{Lane, LaneClose, LaneEnd, LaneMessage, LaneReader};
use demi_plugin_interface::wire::{HostMessage, PluginMessage};
use demi_plugin_interface::{PluginError, PluginId, PortAnswer, PortMessage, Reply, Request};
use tokio::sync::mpsc;

/// What a request sends its lane.
pub(crate) enum Out {
    Call {
        request: u64,
        call: Request,
        events: mpsc::UnboundedSender<Event>,
    },
    Answer {
        request: u64,
        port: u64,
        answer: Result<PortAnswer, String>,
    },
    Cancel {
        request: u64,
    },
    /// The request is over: the lane forgets it.
    Done {
        request: u64,
    },
}

/// What a lane hands a request.
pub(crate) enum Event {
    Port { port: u64, message: PortMessage },
    Reply(Result<Reply, PluginError>),
}

/// The plugin lanes of one serving core: the program's plugins, and the
/// lane of each that is open.
pub struct PluginLanes {
    plugins: Arc<[PluginId]>,
    open: Mutex<HashMap<PluginId, OpenLane>>,
    /// How many lanes were opened for the core.
    opens: AtomicU64,
}

/// An open lane, as the core's requests reach it.
#[derive(Clone)]
pub(crate) struct OpenLane {
    number: u64,
    pub(crate) out: mpsc::UnboundedSender<Out>,
    /// The number of the lane's last request.
    requests: Arc<AtomicU64>,
}

impl OpenLane {
    /// The number of a new request.
    pub(crate) fn next_request(&self) -> u64 {
        self.requests.fetch_add(1, Ordering::Relaxed) + 1
    }
}

/// Why a plugin lane does not open.
pub(crate) enum Unclaimed {
    /// The program declared no plugin of that id.
    Unknown,
    /// Its lane is open already.
    Open,
}

impl PluginLanes {
    pub(crate) fn new(plugins: Arc<[PluginId]>) -> Self {
        Self {
            plugins,
            open: Mutex::default(),
            opens: AtomicU64::new(0),
        }
    }

    fn lanes(&self) -> std::sync::MutexGuard<'_, HashMap<PluginId, OpenLane>> {
        // No section panics while it holds the lock.
        self.open.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The open lane of `plugin`, when one is.
    pub(crate) fn open_lane(&self, plugin: &PluginId) -> Option<OpenLane> {
        self.lanes().get(plugin).cloned()
    }

    /// Takes the lane of `plugin` for an open.
    pub(crate) fn claim(self: &Arc<Self>, plugin: &str) -> Result<Claim, Unclaimed> {
        let plugin = self
            .plugins
            .iter()
            .find(|id| id.as_str() == plugin)
            .cloned()
            .ok_or(Unclaimed::Unknown)?;
        let mut lanes = self.lanes();
        if lanes.contains_key(&plugin) {
            return Err(Unclaimed::Open);
        }
        let number = self.opens.fetch_add(1, Ordering::Relaxed) + 1;
        let (out, calls) = mpsc::unbounded_channel();
        lanes.insert(
            plugin.clone(),
            OpenLane {
                number,
                out,
                requests: Arc::default(),
            },
        );
        Ok(Claim {
            lanes: self.clone(),
            plugin,
            number,
            calls,
        })
    }
}

/// An open of a plugin's lane, from its claim until it ends.
pub(crate) struct Claim {
    lanes: Arc<PluginLanes>,
    plugin: PluginId,
    number: u64,
    calls: mpsc::UnboundedReceiver<Out>,
}

impl Claim {
    /// Carries the core's requests to the program and the program's
    /// messages back, until the program closes the lane, its connection
    /// ends, or the core closes.
    pub(crate) async fn serve(mut self, mut lane: Lane, mut reader: LaneReader) {
        let mut waiting: HashMap<u64, mpsc::UnboundedSender<Event>> = HashMap::new();
        let end = loop {
            tokio::select! {
                out = self.calls.recv() => {
                    // Every request of the lane's holds its sender.
                    let Some(out) = out else {
                        break LaneEnd::Close(LaneClose { code: close_code::AWAY, reason: "core_closing" });
                    };
                    let message = match out {
                        Out::Call { request, call, events } => {
                            waiting.insert(request, events);
                            HostMessage::Call { request, call }
                        }
                        Out::Answer { request, port, answer: Ok(answer) } => HostMessage::Answer { request, port, answer },
                        Out::Answer { request, port, answer: Err(error) } => HostMessage::Unanswered { request, port, error },
                        Out::Cancel { request } if waiting.contains_key(&request) => {
                            HostMessage::Cancel { request, cancel: true }
                        }
                        Out::Cancel { .. } => continue,
                        Out::Done { request } => {
                            waiting.remove(&request);
                            continue;
                        }
                    };
                    let request = message.request();
                    match encode(&message) {
                        Ok(LaneMessage::Text(text)) => {
                            if lane.send(text).await.is_err() {
                                break LaneEnd::Gone;
                            }
                        }
                        Ok(LaneMessage::Binary(bytes)) => {
                            if lane.send_bytes(bytes).await.is_err() {
                                break LaneEnd::Gone;
                            }
                        }
                        // A request the program cannot be sent fails as a
                        // plugin that failed.
                        Err(error) => {
                            tracing::error!(event = "plugin_lane.unencoded", plugin = %self.plugin, %error, "a message of a program's plugin did not encode");
                            if let Some(events) = waiting.remove(&request) {
                                let _ = events.send(Event::Reply(Err(PluginError::failed(format!("the request did not encode: {error}")))));
                            }
                        }
                    }
                }
                message = reader.next() => {
                    let Some(message) = message else {
                        break LaneEnd::Gone;
                    };
                    let message = match decode(message) {
                        Ok(message) => message,
                        Err(error) => {
                            tracing::info!(event = "plugin_lane.unread", plugin = %self.plugin, %error, "a program's plugin sent a message that does not read");
                            break LaneEnd::Close(LaneClose { code: close_code::POLICY, reason: "unexpected_message" });
                        }
                    };
                    match message {
                        PluginMessage::Reply { request, reply } => {
                            if let Some(events) = waiting.get(&request) {
                                let _ = events.send(Event::Reply(Ok(reply)));
                            }
                        }
                        PluginMessage::Error { request, error } => {
                            if let Some(events) = waiting.get(&request) {
                                let _ = events.send(Event::Reply(Err(error)));
                            }
                        }
                        PluginMessage::Port { request, port, message } => {
                            let handed = waiting
                                .get(&request)
                                .is_some_and(|events| events.send(Event::Port { port, message }).is_ok());
                            if !handed {
                                let over = HostMessage::Unanswered { request, port, error: "the request is over".into() };
                                let sent = match encode(&over) {
                                    Ok(LaneMessage::Text(text)) => lane.send(text).await,
                                    Ok(LaneMessage::Binary(bytes)) => lane.send_bytes(bytes).await,
                                    Err(_) => Ok(()),
                                };
                                if sent.is_err() {
                                    break LaneEnd::Gone;
                                }
                            }
                        }
                    }
                }
            }
        };
        // The requests still waiting end as their senders drop.
        drop(waiting);
        lane.end(reader, end);
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        let mut lanes = self.lanes.lanes();
        // A lane opened since this one ended is the next one's.
        if lanes.get(&self.plugin).is_some_and(|open| open.number == self.number) {
            lanes.remove(&self.plugin);
        }
    }
}

trait Numbered {
    fn request(&self) -> u64;
}

impl Numbered for HostMessage {
    fn request(&self) -> u64 {
        match self {
            HostMessage::Call { request, .. }
            | HostMessage::Answer { request, .. }
            | HostMessage::Unanswered { request, .. }
            | HostMessage::Cancel { request, .. } => *request,
        }
    }
}

/// `message` as its lane message: text without bytes, binary with them.
fn encode(message: &HostMessage) -> serde_json::Result<LaneMessage> {
    let (json, buffers) = demi_shared_types::to_json_apart(message)?;
    if buffers.is_empty() {
        return Ok(LaneMessage::Text(json));
    }
    let length = 4 + json.len() + buffers.iter().map(|buffer| 4 + buffer.len()).sum::<usize>();
    let mut bytes = Vec::with_capacity(length);
    for part in std::iter::once(Bytes::from(json)).chain(buffers) {
        let size = u32::try_from(part.len()).map_err(serde::ser::Error::custom)?;
        bytes.extend_from_slice(&size.to_be_bytes());
        bytes.extend_from_slice(&part);
    }
    Ok(LaneMessage::Binary(bytes.into()))
}

/// The program's message of `message`.
fn decode(message: LaneMessage) -> Result<PluginMessage, String> {
    let (json, buffers) = match message {
        LaneMessage::Text(text) => (Bytes::from(text), Vec::new()),
        LaneMessage::Binary(mut bytes) => {
            let mut parts = Vec::new();
            while !bytes.is_empty() {
                if bytes.len() < 4 {
                    return Err("a part's length is cut short".into());
                }
                let size = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
                let _ = bytes.split_to(4);
                if bytes.len() < size {
                    return Err("a part is cut short".into());
                }
                parts.push(bytes.split_to(size));
            }
            let mut parts = parts.into_iter();
            let json = parts.next().ok_or("a binary message holds no JSON")?;
            (json, parts.collect())
        }
    };
    demi_shared_types::from_json_apart(&json, buffers).map_err(|error| error.to_string())
}

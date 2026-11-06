//! A program's plugins: the program declares them with `PUT /plugins`
//! before its first core opens, and each core's instance of one sends its
//! requests to the program on the plugin's lane (`crates/core-api/src/plugin_lanes.rs`).
//!
//! ```text
//! core's thread                        lane's task                program
//! LanePlugin::call --- Out::Call ----> { request, call } -------->
//!   port.relay   <---- Event::Port --- { request, port, message } <--
//!   ------------------ Out::Answer --> { request, port, answer } ->
//!   reply        <---- Event::Reply -- { request, reply } <--------
//! ```
//!
//! A request runs on its core's thread, where its port is; the lane's task
//! only carries messages. A request while the plugin's lane is not open
//! fails. A page call's task keeps its port after its reply, until the
//! instance ends, as the contract allows.

use std::rc::Rc;
use std::sync::Arc;

use demi_plugin_interface::{
    Manifest, Plugin, PluginError, PluginFactory, PluginId, PluginPort, PortMessage, Reply, Request, UserId,
};
use futures_util::future::LocalBoxFuture;
use tokio::sync::mpsc;
use tokio_util::sync::{CancellationToken, DropGuard};

use super::cores::OpenCores;
use crate::plugin_lanes::{Event, OpenLane, Out};

/// A program's plugin, as the process registers it.
pub(super) struct ProgramPlugin {
    manifest: Manifest,
    cores: Arc<OpenCores>,
}

impl ProgramPlugin {
    pub(super) fn new(manifest: Manifest, cores: Arc<OpenCores>) -> Self {
        Self { manifest, cores }
    }
}

impl PluginFactory for ProgramPlugin {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn instance(&self, user: &UserId) -> Rc<dyn Plugin> {
        let ended = CancellationToken::new();
        Rc::new(LanePlugin {
            core: user.clone(),
            plugin: self.manifest.id.clone(),
            cores: self.cores.clone(),
            ended: ended.clone(),
            _ends: ended.drop_guard(),
        })
    }
}

/// A program plugin's instance in one core: each request goes to the
/// program on the plugin's lane of the core's connection.
struct LanePlugin {
    core: UserId,
    plugin: PluginId,
    cores: Arc<OpenCores>,
    /// Cancelled as the instance ends, which ends the tasks it started.
    ended: CancellationToken,
    _ends: DropGuard,
}

impl Plugin for LanePlugin {
    fn call(&self, request: Request, port: PluginPort) -> LocalBoxFuture<'_, Result<Reply, PluginError>> {
        Box::pin(async move {
            let lane = self
                .cores
                .serving(&self.core)
                .and_then(|serving| serving.plugin_lanes.open_lane(&self.plugin))
                .ok_or_else(|| {
                    PluginError::failed(format!("the program has no lane of the plugin \"{}\" open", self.plugin))
                })?;
            let number = lane.next_request();
            let keeps_port = matches!(request, Request::PageCall { .. });
            let (events, mut received) = mpsc::unbounded_channel();
            if lane.out.send(Out::Call { request: number, call: request, events }).is_err() {
                return Err(lane_ended());
            }
            let mut call = Called {
                lane: lane.clone(),
                request: number,
                over: false,
            };
            let mut stopped = false;
            let reply = loop {
                tokio::select! {
                    event = received.recv() => match event {
                        Some(Event::Port { port: id, message }) => {
                            answer(&lane, &port, number, id, message, &self.ended);
                        }
                        Some(Event::Reply(reply)) => break reply,
                        None => break Err(lane_ended()),
                    },
                    // The program hears that the request is stopped, and
                    // replies when its work has ended.
                    () = port.cancelled(), if !stopped => {
                        stopped = true;
                        let _ = lane.out.send(Out::Cancel { request: number });
                    }
                }
            };
            call.over = true;
            if !(keeps_port && reply.is_ok()) {
                let _ = lane.out.send(Out::Done { request: number });
                return reply;
            }
            // A page call's task keeps its port, until the instance ends or
            // the lane does.
            let ended = self.ended.clone();
            tokio::task::spawn_local(async move {
                loop {
                    tokio::select! {
                        event = received.recv() => match event {
                            Some(Event::Port { port: id, message }) => answer(&lane, &port, number, id, message, &ended),
                            _ => break,
                        },
                        () = ended.cancelled() => break,
                    }
                }
                let _ = lane.out.send(Out::Done { request: number });
            });
            reply
        })
    }
}

/// Passes the port message `id` of request `request` to `port`, and its
/// answer back, in a task that `ended` ends.
fn answer(lane: &OpenLane, port: &PluginPort, request: u64, id: u64, message: PortMessage, ended: &CancellationToken) {
    let (lane, port, ended) = (lane.clone(), port.clone(), ended.clone());
    tokio::task::spawn_local(async move {
        let answer = tokio::select! {
            answer = port.relay(message) => answer.map_err(|error| error.to_string()),
            () = ended.cancelled() => return,
        };
        let _ = lane.out.send(Out::Answer { request, port: id, answer });
    });
}

/// A request in flight: dropping it before its reply, as when the plugin
/// host stops waiting, tells the program and forgets it.
struct Called {
    lane: OpenLane,
    request: u64,
    over: bool,
}

impl Drop for Called {
    fn drop(&mut self) {
        if !self.over {
            let _ = self.lane.out.send(Out::Cancel { request: self.request });
            let _ = self.lane.out.send(Out::Done { request: self.request });
        }
    }
}

fn lane_ended() -> PluginError {
    PluginError::Ended {
        message: "the program's plugin lane ended".into(),
    }
}


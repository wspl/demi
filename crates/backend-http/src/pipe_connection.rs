//! `WS /api/runner/pipes`: a runner's pipe connection, whose lanes carry the
//! stream pipes it writes (`crates/runner-protocol/src/wire/pipe_connection.rs`).
//!
//! The upgrade checks the device token as every pipe route does. The
//! connection then runs in a reader, this task, and a writer:
//!
//! ```text
//!  runner --> reader --- each lane's messages ---> its pipe's relay
//!              |  each open claims its pipe in a task of its own
//!              v
//!  runner <-- writer <-- opened, refused, credit; each relay's sink and close
//! ```
//!
//! - A lane opens once its pipe's source end is claimed, as the pipe's own
//!   route claimed it, and is refused with the status and words that route
//!   answered. What the runner sends meanwhile waits, within the lane's
//!   credit, and its relay takes it first.
//! - Each relay takes the lane's messages as its pipe takes them, and the
//!   reader grants the runner one more message for each it took.
//! - A message past a lane's credit, or larger than a stream pipe's, closes
//!   that lane's pipe as failed; a frame that breaks the format ends the
//!   connection. The connection's end ends every lane's pipe.

use std::collections::HashMap;

use axum::body::Bytes;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade, close_code};
use axum::http::StatusCode;
use axum::response::Response;
use axum_extra::TypedHeader;
use axum_extra::headers::Authorization;
use axum_extra::headers::authorization::Bearer;
use demi_backend_database::devices::DeviceRecord;
use demi_runner_client::{DeviceSource, FromSource, ToSource, relay_stream_pipe};
use demi_runner_protocol::wire::{PIPE_LANE_CREDIT, PipeLaneRefusal, PipeLaneTarget, STREAM_PIPE_MESSAGE_BYTES};
use demi_shared_lanes::{Assembly, AssemblyError, Frame, LANE_PIECE_BYTES, Message as LaneMessage, pieces};
use futures_util::{SinkExt as _, StreamExt as _};
use tokio::sync::mpsc;
use tokio::task::AbortHandle;
use tracing::Instrument as _;

use super::AppState;
use super::runners::{claim_source, device_of};

/// The most bytes of one message the runner sends on the connection: a
/// piece and its header, or an open and its target, with room to spare.
/// Bounded because a message is read whole into memory.
const MAX_FRAME_BYTES: usize = LANE_PIECE_BYTES + 1024;

/// `WS /api/runner/pipes`: a missing or unknown device token answers 401
/// before the upgrade.
pub(super) async fn connect(
    State(state): State<AppState>,
    authorization: Option<TypedHeader<Authorization<Bearer>>>,
    upgrade: WebSocketUpgrade,
) -> Response {
    let device = match device_of(&state, authorization).await {
        Ok(device) => device,
        Err(refusal) => return *refusal,
    };
    let span = tracing::Span::current();
    upgrade
        .max_message_size(MAX_FRAME_BYTES)
        .max_frame_size(MAX_FRAME_BYTES)
        .on_upgrade(move |socket| serve(socket, state, device).instrument(span))
}

/// A lane of the connection, as the reader holds it.
struct Lane {
    phase: Phase,
    assembling: Assembly,
    /// The runner's messages its relay has not taken yet, at most the credit.
    held: u16,
    /// Whether the lane's messages end here: the runner closed it, or broke
    /// its rules, and the relay heard the close.
    closed: bool,
}

enum Phase {
    /// Its pipe is being claimed; what the runner sent meanwhile waits.
    Opening { task: AbortHandle, waiting: Vec<FromSource> },
    Open { to_relay: mpsc::UnboundedSender<FromSource> },
}

/// What a lane's relay tells the reader.
enum Told {
    /// It took one of the runner's messages.
    Took(u32),
    /// Its pipe ended.
    Ended(u32),
}

struct Connection {
    state: AppState,
    device: DeviceRecord,
    lanes: HashMap<u32, Lane>,
    /// The number of the last lane the runner opened.
    last_lane: u32,
    to_writer: mpsc::UnboundedSender<Bytes>,
    claimed: mpsc::UnboundedSender<(u32, Result<DeviceSource, (StatusCode, String)>)>,
    told: mpsc::UnboundedSender<Told>,
    opened: u64,
    refused: u64,
}

async fn serve(socket: WebSocket, state: AppState, device: DeviceRecord) {
    let (mut sink, mut stream) = socket.split();
    let (to_writer, mut writes) = mpsc::unbounded_channel::<Bytes>();
    let mut writer = tokio::spawn(async move {
        while let Some(frame) = writes.recv().await {
            if sink.send(Message::Binary(frame)).await.is_err() {
                return;
            }
        }
    });
    let (claimed, mut claims) = mpsc::unbounded_channel();
    let (told, mut tellings) = mpsc::unbounded_channel();
    tracing::info!(event = "pipe_connection.opened", device = %device.id, "a runner's pipe connection opened");
    let mut connection = Connection {
        state,
        device,
        lanes: HashMap::new(),
        last_lane: 0,
        to_writer,
        claimed,
        told,
        opened: 0,
        refused: 0,
    };
    let ending = loop {
        tokio::select! {
            _ = &mut writer => break "write failed",
            message = stream.next() => match message {
                Some(Ok(Message::Binary(bytes))) => {
                    if let Err(why) = connection.read(bytes) {
                        break why;
                    }
                }
                Some(Ok(Message::Text(_))) => break "text message",
                // Pings are answered by the socket itself.
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                Some(Ok(Message::Close(_)) | Err(_)) | None => break "runner closed",
            },
            Some((lane, claim)) = claims.recv() => connection.answered(lane, claim),
            Some(told) = tellings.recv() => connection.told(told),
        }
    };
    let open = connection.lanes.len();
    // Each relay hears its lane end, which fails its pipe; a claim still
    // made stops.
    for lane in connection.lanes.into_values() {
        if let Phase::Opening { task, .. } = lane.phase {
            task.abort();
        }
    }
    writer.abort();
    tracing::info!(
        event = "pipe_connection.ended",
        device = %connection.device.id,
        reason = ending,
        lanes_opened = connection.opened,
        lanes_refused = connection.refused,
        lanes_open = open,
        "a runner's pipe connection ended"
    );
}

impl Connection {
    /// Reads one frame of the runner's; an error is how it broke the format,
    /// which ends the connection.
    fn read(&mut self, bytes: Bytes) -> Result<(), &'static str> {
        match Frame::decode(bytes)? {
            Frame::Open { lane, target } => self.open(lane, &target),
            Frame::Data { lane, flags, piece } => {
                let Some(open) = self.lanes.get_mut(&lane) else {
                    // A message for a lane whose pipe ended goes nowhere.
                    return if lane <= self.last_lane { Ok(()) } else { Err("data for a lane never opened") };
                };
                if open.closed {
                    return Ok(());
                }
                let message = match open.assembling.take(flags, piece, STREAM_PIPE_MESSAGE_BYTES) {
                    Ok(Some(message)) => message,
                    Ok(None) => return Ok(()),
                    Err(AssemblyError::TooBig) => {
                        self.break_lane(lane, close_code::SIZE, "message_too_big");
                        return Ok(());
                    }
                    Err(AssemblyError::NotUtf8) => {
                        self.break_lane(lane, close_code::INVALID, "not_utf8");
                        return Ok(());
                    }
                };
                if open.held >= PIPE_LANE_CREDIT {
                    self.break_lane(lane, close_code::POLICY, "credit_exceeded");
                    return Ok(());
                }
                open.held += 1;
                let message = match message {
                    LaneMessage::Binary(bytes) => FromSource::Binary(bytes),
                    LaneMessage::Text(text) => FromSource::Text(text),
                };
                self.deliver(lane, message);
                Ok(())
            }
            Frame::Close { lane, code, reason } => {
                match self.lanes.get_mut(&lane) {
                    Some(open) if !open.closed => {
                        open.closed = true;
                        self.deliver(lane, FromSource::Close { code, reason });
                    }
                    Some(_) => {}
                    None if lane <= self.last_lane => {}
                    None => return Err("close of a lane never opened"),
                }
                Ok(())
            }
            Frame::Connection(_) | Frame::Opened { .. } | Frame::Refused { .. } | Frame::Credit { .. } => {
                Err("unknown message kind")
            }
        }
    }

    fn write(&self, frame: Frame) {
        // A writer that ended ends the connection, which its task's end says.
        let _ = self.to_writer.send(frame.encode());
    }

    /// Claims the pipe `target` names for lane `lane`, in a task of its own.
    fn open(&mut self, lane: u32, target: &[u8]) -> Result<(), &'static str> {
        if lane <= self.last_lane {
            return Err("lane number not new");
        }
        self.last_lane = lane;
        let pipe = match serde_json::from_slice::<PipeLaneTarget>(target) {
            Ok(PipeLaneTarget::StreamPipe { pipe }) => pipe,
            Err(error) => {
                self.refuse(lane, StatusCode::BAD_REQUEST, format!("The lane's target: {error}"));
                return Ok(());
            }
        };
        let state = self.state.clone();
        let device = self.device.clone();
        let claimed = self.claimed.clone();
        let task = tokio::spawn(
            async move {
                let claim = claim_source(&state, pipe, device).await;
                // A connection that ended takes no claim, and the source it
                // drops fails its pipe.
                let _ = claimed.send((lane, claim));
            }
            .in_current_span(),
        );
        self.lanes.insert(
            lane,
            Lane {
                phase: Phase::Opening {
                    task: task.abort_handle(),
                    waiting: Vec::new(),
                },
                assembling: Assembly::default(),
                held: 0,
                closed: false,
            },
        );
        Ok(())
    }

    fn refuse(&mut self, lane: u32, status: StatusCode, message: String) {
        self.refused += 1;
        let refusal = PipeLaneRefusal {
            status: status.as_u16(),
            message,
        };
        // A refusal serializes as a JSON object of a number and a string,
        // which serde_json never refuses.
        let refusal = Bytes::from(serde_json::to_vec(&refusal).expect("a lane's refusal serializes"));
        self.write(Frame::Refused { lane, refusal });
    }

    /// Opens a lane whose pipe was claimed, with a relay that takes first
    /// what the runner sent meanwhile, or refuses it.
    fn answered(&mut self, lane: u32, claim: Result<DeviceSource, (StatusCode, String)>) {
        let Some(open) = self.lanes.get_mut(&lane) else {
            return;
        };
        let Phase::Opening { waiting, .. } = &mut open.phase else {
            return;
        };
        let waiting = std::mem::take(waiting);
        let source = match claim {
            Ok(source) => source,
            Err((status, message)) => {
                self.lanes.remove(&lane);
                self.refuse(lane, status, message);
                return;
            }
        };
        let (to_relay, from_runner) = mpsc::unbounded_channel();
        for message in waiting {
            let _ = to_relay.send(message);
        }
        open.phase = Phase::Open { to_relay };
        self.opened += 1;
        // Ahead of anything the relay says, which goes through the same
        // writer.
        self.write(Frame::Opened { lane });
        let told = self.told.clone();
        let to_writer = self.to_writer.clone();
        tokio::spawn(
            async move {
                let took = told.clone();
                let from_runner = futures_util::stream::unfold(from_runner, move |mut from_runner| {
                    let took = took.clone();
                    async move {
                        let message = from_runner.recv().await?;
                        if matches!(message, FromSource::Binary(_) | FromSource::Text(_)) {
                            // A connection that ended grants nothing more.
                            let _ = took.send(Told::Took(lane));
                        }
                        Some((message, from_runner))
                    }
                });
                let to_runner = |said: ToSource| {
                    let frame = match said {
                        ToSource::Text(text) => {
                            return pieces(lane, false, Bytes::from(text))
                                .all(|piece| to_writer.send(piece.encode()).is_ok());
                        }
                        ToSource::Close { code, reason } => Frame::Close { lane, code, reason },
                    };
                    to_writer.send(frame.encode()).is_ok()
                };
                relay_stream_pipe(source, Box::pin(from_runner), to_runner).await;
                let _ = told.send(Told::Ended(lane));
            }
            .in_current_span(),
        );
    }

    /// Hands lane `lane`'s message to its relay, or keeps it for the relay
    /// while its pipe is claimed.
    fn deliver(&mut self, lane: u32, message: FromSource) {
        let Some(open) = self.lanes.get_mut(&lane) else {
            return;
        };
        match &mut open.phase {
            Phase::Opening { waiting, .. } => waiting.push(message),
            // A relay that ended takes nothing more.
            Phase::Open { to_relay } => {
                let _ = to_relay.send(message);
            }
        }
    }

    /// Fails lane `lane`'s pipe for a message that broke its rules: its relay
    /// hears the close, and answers the runner with the pipe's failure.
    fn break_lane(&mut self, lane: u32, code: u16, reason: &str) {
        if let Some(open) = self.lanes.get_mut(&lane) {
            open.closed = true;
            open.assembling.clear();
        }
        self.deliver(
            lane,
            FromSource::Close {
                code,
                reason: format!("the runner broke the lane's rules: {reason}"),
            },
        );
    }

    fn told(&mut self, told: Told) {
        match told {
            Told::Took(lane) => {
                if let Some(open) = self.lanes.get_mut(&lane) {
                    open.held = open.held.saturating_sub(1);
                    if !open.closed {
                        self.write(Frame::Credit { lane, messages: 1 });
                    }
                }
            }
            Told::Ended(lane) => {
                self.lanes.remove(&lane);
            }
        }
    }
}

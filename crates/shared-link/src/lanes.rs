//! The link's lanes: which stream each message goes on, and the tasks that
//! write and read those streams.
//!
//! ```text
//!   Outgoing::send ─► lane entry ─► writer task ─► yamux stream ─► peer
//!                     (queue)
//!   peer ─► yamux stream ─► reader task ─► Incoming
//! ```
//!
//! A command or a request is a stream for its whole life, and a lane has a
//! stream, not a series of them: its [`Entry`] is made by the end that sends
//! first (which opens the stream) or by the end that accepts the stream, and
//! it ends when both halves of the stream have ended. A writer ends its half
//! with the message its route calls the last, or, for a command, once the
//! peer has said all it will; a lane that two ends opened at once has a stream
//! each way, and each end writes on its own.
//!
//! The messages of one lane wait in an unbounded queue that only
//! [`Shared::send`] bounds, by a few places a sender takes and the writer
//! gives back once the message is written. So [`Shared::post`], which cannot
//! wait, queues behind what was sent before it, in order.
//!
//! A message with no lane to join is dropped when it belongs to a command that
//! ended (the link remembers the last few, since the commands' own users are
//! the ones that know), and goes on a stream of its own when it is a
//! request's reply that has no stream left or a request nobody answers.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    io,
    sync::{Arc, PoisonError, Weak, atomic::Ordering},
};

use demi_runner_protocol::wire::{Frame, Lane, LaneKind, MAX_MESSAGE_BYTES, Mode, Route};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    sync::{OwnedSemaphorePermit, Semaphore, mpsc, TryAcquireError},
};
use tokio_util::{compat::FuturesAsyncReadCompatExt as _, sync::CancellationToken};

use crate::{
    connection::{
        EndingRead, Held, INBOUND_STREAMS, ReadSide, STREAM_AHEAD, Shared, StreamLife, WINDOW_COMMAND,
        WriteSide, YStream,
    },
    framing,
};

/// The places a sender takes in a lane's queue: a sender waits when they are
/// all taken, until the writer has written a message.
pub(crate) const LANE_QUEUE: usize = 8;
/// How many ended commands the link remembers, so that a message that comes
/// after a command's end is dropped rather than starting a stream for it.
const ENDED_COMMANDS: usize = 256;
/// The largest message a writer joins to its header in one write.
const JOINED_BYTES: usize = 64 * 1024;

/// A message waiting for its lane's writer.
pub(crate) struct Queued {
    message: Vec<u8>,
    last: bool,
    _room: Option<OwnedSemaphorePermit>,
}

/// The lanes of a link that have a stream, and the commands that ended.
#[derive(Default)]
pub(crate) struct Lanes {
    live: HashMap<Lane, Arc<Entry>>,
    ended: Ended,
}

impl Lanes {
    /// How many lanes have a stream.
    #[cfg(feature = "testing")]
    pub(crate) fn records(&self) -> usize {
        self.live.len()
    }
}

#[derive(Default)]
struct Ended {
    order: VecDeque<Lane>,
    set: HashSet<Lane>,
}

impl Ended {
    fn remember(&mut self, lane: Lane) {
        if !self.set.insert(lane.clone()) {
            return;
        }
        self.order.push_back(lane);
        if self.order.len() > ENDED_COMMANDS
            && let Some(oldest) = self.order.pop_front()
        {
            self.set.remove(&oldest);
        }
    }

    fn contains(&self, lane: &Lane) -> bool {
        self.set.contains(lane)
    }
}

/// A lane with a stream.
pub(crate) struct Entry {
    token: u64,
    queue: mpsc::UnboundedSender<Queued>,
    room: Arc<Semaphore>,
    /// Cancelled once a stream of the lane the peer wrote on has ended after
    /// it wrote: the peer said all it will.
    peer_done: CancellationToken,
    life: Weak<LaneLife>,
}

impl Entry {
    /// Whether the lane's writer still takes messages.
    fn takes(&self) -> bool {
        !self.queue.is_closed() && !self.room.is_closed()
    }
}

/// What a lane's tasks hold together: the lane's entry goes when the last of
/// them ends, and a command that ended is remembered.
pub(crate) struct LaneLife {
    shared: Arc<Shared>,
    lane: Lane,
    token: u64,
}

impl Drop for LaneLife {
    fn drop(&mut self) {
        let mut lanes = self.shared.lanes.lock().unwrap_or_else(PoisonError::into_inner);
        if lanes
            .live
            .get(&self.lane)
            .is_some_and(|entry| entry.token == self.token)
        {
            lanes.live.remove(&self.lane);
            if self.lane.kind == LaneKind::Command {
                lanes.ended.remember(self.lane.clone());
            }
        }
    }
}

/// A lane just made: what its tasks hold, and the end of its queue.
struct Made {
    life: Arc<LaneLife>,
    entry: Arc<Entry>,
    queue: mpsc::UnboundedReceiver<Queued>,
}

fn make(shared: &Arc<Shared>, lanes: &mut Lanes, lane: &Lane) -> Made {
    let (sender, queue) = mpsc::unbounded_channel();
    let token = shared.next_token.fetch_add(1, Ordering::Relaxed);
    let life = Arc::new(LaneLife {
        shared: shared.clone(),
        lane: lane.clone(),
        token,
    });
    let entry = Arc::new(Entry {
        token,
        queue: sender,
        room: Arc::new(Semaphore::new(LANE_QUEUE)),
        peer_done: CancellationToken::new(),
        life: Arc::downgrade(&life),
    });
    lanes.live.insert(lane.clone(), entry.clone());
    Made { life, entry, queue }
}

/// Where a message goes.
enum Target {
    Entry(Arc<Entry>),
    /// On a stream of its own.
    Tell,
    /// Nowhere: the command it belongs to ended.
    Drop,
}

impl Shared {
    fn target(self: &Arc<Self>, route: &Route) -> Target {
        if route.mode == Mode::Tell {
            return Target::Tell;
        }
        let mut lanes = self.lanes.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(entry) = lanes.live.get(&route.lane) {
            if entry.takes() {
                return Target::Entry(entry.clone());
            }
            return match route.mode {
                Mode::Stream => Target::Drop,
                _ => Target::Tell,
            };
        }
        if route.mode == Mode::Answer {
            return Target::Tell;
        }
        if route.lane.kind == LaneKind::Command && lanes.ended.contains(&route.lane) {
            return Target::Drop;
        }
        let Made { life, entry, queue } = make(self, &mut lanes, &route.lane);
        drop(lanes);
        self.tasks.spawn(write_lane(
            self.clone(),
            route.lane.clone(),
            life,
            entry.clone(),
            queue,
            None,
        ));
        Target::Entry(entry)
    }

    /// Queues `frame` on its lane once there is room; the frame comes back
    /// when the link is over.
    pub(crate) async fn send(self: &Arc<Self>, frame: Frame) -> Result<(), Frame> {
        let (mut message, route) = frame.into_parts();
        loop {
            if self.is_closed() {
                return Err(Frame::from_parts(message, route));
            }
            match self.target(&route) {
                Target::Drop => return Ok(()),
                Target::Tell => {
                    let permit = tokio::select! {
                        () = self.closed.cancelled() => None,
                        permit = self.outbound.clone().acquire_owned() => permit.ok(),
                    };
                    let Some(permit) = permit else {
                        return Err(Frame::from_parts(message, route));
                    };
                    self.tell(permit, message, route.lane);
                    return Ok(());
                }
                Target::Entry(entry) => {
                    let room = tokio::select! {
                        () = self.closed.cancelled() => {
                            return Err(Frame::from_parts(message, route));
                        }
                        room = entry.room.clone().acquire_owned() => room,
                    };
                    // A closed lane's writer ended: the next look at the lane
                    // says what its messages do now.
                    let Ok(room) = room else { continue };
                    match entry.queue.send(Queued {
                        message,
                        last: route.last,
                        _room: Some(room),
                    }) {
                        Ok(()) => return Ok(()),
                        Err(back) => message = back.0.message,
                    }
                }
            }
        }
    }

    /// Queues `frame` if its lane has room now; the frame comes back when it
    /// has none, or, with `true`, when the link is over.
    pub(crate) fn try_send(self: &Arc<Self>, frame: Frame) -> Result<(), (Frame, bool)> {
        let (mut message, route) = frame.into_parts();
        loop {
            if self.is_closed() {
                return Err((Frame::from_parts(message, route), true));
            }
            match self.target(&route) {
                Target::Drop => return Ok(()),
                Target::Tell => match self.outbound.clone().try_acquire_owned() {
                    Ok(permit) => {
                        self.tell(permit, message, route.lane);
                        return Ok(());
                    }
                    Err(TryAcquireError::NoPermits) => {
                        return Err((Frame::from_parts(message, route), false));
                    }
                    Err(TryAcquireError::Closed) => {
                        return Err((Frame::from_parts(message, route), true));
                    }
                },
                Target::Entry(entry) => {
                    let room = match entry.room.clone().try_acquire_owned() {
                        Ok(room) => room,
                        Err(TryAcquireError::NoPermits) => {
                            return Err((Frame::from_parts(message, route), false));
                        }
                        Err(TryAcquireError::Closed) => continue,
                    };
                    match entry.queue.send(Queued {
                        message,
                        last: route.last,
                        _room: Some(room),
                    }) {
                        Ok(()) => return Ok(()),
                        Err(back) => message = back.0.message,
                    }
                }
            }
        }
    }

    /// Queues `frame` for a caller that cannot wait, such as a drop: behind
    /// what its lane already holds, whether or not the lane has room.
    pub(crate) fn post(self: &Arc<Self>, frame: Frame) {
        let (mut message, route) = frame.into_parts();
        loop {
            if self.is_closed() {
                return;
            }
            match self.target(&route) {
                Target::Drop => return,
                Target::Tell => {
                    let shared = self.clone();
                    if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                        runtime.spawn(async move {
                            let permit = tokio::select! {
                                () = shared.closed.cancelled() => return,
                                permit = shared.outbound.clone().acquire_owned() => permit,
                            };
                            if let Ok(permit) = permit {
                                shared.tell(permit, message, route.lane);
                            }
                        });
                    }
                    return;
                }
                Target::Entry(entry) => match entry.queue.send(Queued {
                    message,
                    last: route.last,
                    _room: None,
                }) {
                    Ok(()) => return,
                    Err(back) => message = back.0.message,
                },
            }
        }
    }

    /// Sends one message on a stream of its own, which ends with it: the
    /// peer ends its half at once, and the stream is gone when both halves
    /// are.
    fn tell(self: &Arc<Self>, permit: OwnedSemaphorePermit, message: Vec<u8>, lane: Lane) {
        let shared = self.clone();
        self.tasks.spawn(async move {
            let life = StreamLife::opened(Some(permit));
            let Ok(stream) = shared.open_raw(None).await else {
                return;
            };
            let (read, mut write) = tokio::io::split(stream.compat());
            let mut bytes = framing::lane_header(&lane, true);
            bytes.extend_from_slice(&framing::frame_header(message.len()));
            bytes.extend_from_slice(&message);
            let written = tokio::select! {
                () = shared.closed.cancelled() => return,
                written = async {
                    write.write_all(&bytes).await?;
                    write.shutdown().await
                } => written,
            };
            if written.is_ok() {
                let read = EndingRead::new(read, &shared);
                shared
                    .readers
                    .spawn(read_stream(shared.clone(), read, None, life));
            }
        });
    }
}

/// Serves a stream the other end opened.
pub(crate) fn accept(shared: &Arc<Shared>, stream: YStream) {
    if shared.inbound.fetch_add(1, Ordering::AcqRel) >= INBOUND_STREAMS {
        shared.inbound.fetch_sub(1, Ordering::AcqRel);
        shared.violation("the peer opened more streams than a link has".into());
        return;
    }
    let life = StreamLife::accepted(shared);
    shared
        .readers
        .spawn(serve_accepted(shared.clone(), stream, life));
}

async fn serve_accepted(shared: Arc<Shared>, stream: YStream, life: Arc<StreamLife>) {
    let mut head = EndingRead::new(stream.compat(), &shared);
    let header = tokio::select! {
        () = shared.abort.cancelled() => return,
        header = framing::read_lane(&mut head) => header,
    };
    let (lane, one_way) = match header {
        Ok(header) => header,
        Err(error) => {
            if error.kind() == io::ErrorKind::InvalidData {
                shared.violation(error.to_string());
            }
            return;
        }
    };
    let mut stream = head.into_inner();
    // The window this end receives a command's input with, announced now
    // that the lane says it is a command's; it acknowledges every stream, so
    // the peer's count of streams that wait for it does not fill.
    stream
        .get_mut()
        .raise_receive_window(if lane.kind == LaneKind::Command { WINDOW_COMMAND } else { 0 });
    let (read, mut write) = tokio::io::split(stream);
    let read = EndingRead::new(read, &shared);
    if lane.kind == LaneKind::Heartbeat {
        // The peer's probes end with the link, not before it: no drain of a
        // close waits for them.
        let echoing = shared.clone();
        tokio::spawn(async move { echo(&echoing, read, write, life).await });
        return;
    }
    if one_way {
        // A request nobody answers: this end has nothing to write back.
        let _ = write.shutdown().await;
        read_stream(shared, read, None, life).await;
        return;
    }
    enum Join {
        /// The lane has a stream already, which this end writes on.
        Present(Arc<Entry>, Arc<LaneLife>),
        New(Made),
        /// A command that ended, whose stream a stray message opened.
        Ended,
    }
    let join = {
        let mut lanes = shared.lanes.lock().unwrap_or_else(PoisonError::into_inner);
        let present = lanes
            .live
            .get(&lane)
            .and_then(|entry| entry.life.upgrade().map(|life| (entry.clone(), life)));
        match present {
            Some((entry, life)) => Join::Present(entry, life),
            None if lane.kind == LaneKind::Command && lanes.ended.contains(&lane) => Join::Ended,
            None => Join::New(make(&shared, &mut lanes, &lane)),
        }
    };
    let (life_of_lane, peer_done) = match join {
        Join::Ended => {
            let _ = write.shutdown().await;
            return;
        }
        Join::Present(entry, lane_life) => {
            // The lane writes on a stream of its own, so this one carries
            // nothing back: its half ends at once, or the peer's reader of it
            // would wait for the end of the link.
            let _ = write.shutdown().await;
            (lane_life, entry.peer_done.clone())
        }
        Join::New(Made { life: lane_life, entry, queue }) => {
            let peer_done = entry.peer_done.clone();
            shared.tasks.spawn(write_lane(
                shared.clone(),
                lane.clone(),
                lane_life.clone(),
                entry,
                queue,
                Some((write, life.clone())),
            ));
            (lane_life, peer_done)
        }
    };
    read_stream(shared, read, Some((life_of_lane, peer_done)), life).await;
}

/// Echoes the probes of the peer on the stream it opened for them.
async fn echo(shared: &Arc<Shared>, mut read: ReadSide, mut write: WriteSide, _life: Arc<StreamLife>) {
    let mut byte = [0u8; 1];
    loop {
        let read = tokio::select! {
            () = shared.abort.cancelled() => return,
            read = read.read(&mut byte) => read,
        };
        if !matches!(read, Ok(1)) {
            return;
        }
        let sent = tokio::select! {
            () = shared.abort.cancelled() => return,
            sent = write.write_all(&byte) => sent,
        };
        if sent.is_err() {
            return;
        }
    }
}

/// Reads a stream's messages into the link until it ends.
async fn read_stream(
    shared: Arc<Shared>,
    mut read: ReadSide,
    lane: Option<(Arc<LaneLife>, CancellationToken)>,
    _life: Arc<StreamLife>,
) {
    let ahead = Arc::new(Semaphore::new(STREAM_AHEAD));
    let Some(messages) = shared
        .messages
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
    else {
        return;
    };
    let mut frames = 0usize;
    loop {
        // The application holds this stream's messages up to a number; the
        // stream is read no further while it does.
        let place = tokio::select! {
            () = shared.abort.cancelled() => return,
            place = ahead.clone().acquire_owned() => match place {
                Ok(place) => place,
                Err(_) => return,
            },
        };
        let length = tokio::select! {
            () = shared.abort.cancelled() => return,
            length = framing::read_length(&mut read, MAX_MESSAGE_BYTES) => length,
        };
        let length = match length {
            Ok(Some(length)) => length,
            Ok(None) => {
                // The peer said all it will on this stream; when it said
                // something, its lane is done with it.
                if frames > 0
                    && let Some((_, done)) = &lane
                {
                    done.cancel();
                }
                return;
            }
            Err(error) => {
                shared.read_failed(&error);
                return;
            }
        };
        let budget = tokio::select! {
            () = shared.abort.cancelled() => return,
            budget = shared.budget.clone().acquire_many_owned(length as u32) => match budget {
                Ok(budget) => budget,
                Err(_) => return,
            },
        };
        let message = tokio::select! {
            () = shared.abort.cancelled() => return,
            message = framing::read_message(&mut read, length) => message,
        };
        let message = match message {
            Ok(message) => message,
            Err(error) => {
                shared.read_failed(&error);
                return;
            }
        };
        frames += 1;
        let sent = tokio::select! {
            () = shared.abort.cancelled() => return,
            sent = messages.send(Ok((
                message,
                Held {
                    _budget: Some(budget),
                    _stream: Some(place),
                },
            ))) => sent,
        };
        if sent.is_err() {
            return;
        }
    }
}

/// Writes a lane's queue to its stream, opening the stream with the first
/// message unless it has one, until the lane sends its last message, a
/// command's peer has said all it will, or the link ends.
async fn write_lane(
    shared: Arc<Shared>,
    lane: Lane,
    life: Arc<LaneLife>,
    entry: Arc<Entry>,
    mut queue: mpsc::UnboundedReceiver<Queued>,
    attached: Option<(WriteSide, Arc<StreamLife>)>,
) {
    let mut stream = attached;
    let mut failed = false;
    let mut draining = false;
    // A command is over when its peer says so; a request is answered by one
    // message, which says so itself.
    let follows_peer = lane.kind == LaneKind::Command;
    loop {
        let queued = tokio::select! {
            biased;
            () = shared.closed.cancelled() => break,
            () = shared.closing.cancelled() => {
                draining = true;
                break;
            }
            () = async {
                if follows_peer {
                    entry.peer_done.cancelled().await;
                } else {
                    std::future::pending::<()>().await;
                }
            } => break,
            queued = queue.recv() => queued,
        };
        let Some(queued) = queued else { break };
        let last = queued.last;
        if write_message(&shared, &lane, &life, &entry, &mut stream, &queued)
            .await
            .is_err()
        {
            failed = true;
            break;
        }
        if last {
            break;
        }
    }
    entry.room.close();
    queue.close();
    // What was queued before the link was closed in order still goes.
    if draining {
        while let Some(queued) = queue.recv().await {
            if failed || shared.closed.is_cancelled() {
                continue;
            }
            let last = queued.last;
            if write_message(&shared, &lane, &life, &entry, &mut stream, &queued)
                .await
                .is_err()
            {
                failed = true;
            }
            if last {
                break;
            }
        }
    }
    if let Some((mut write, _life)) = stream {
        let _ = tokio::select! {
            () = shared.closed.cancelled() => Ok(()),
            result = write.shutdown() => result,
        };
    }
}

/// Writes one message, opening the lane's stream first when it has none.
async fn write_message(
    shared: &Arc<Shared>,
    lane: &Lane,
    life: &Arc<LaneLife>,
    entry: &Arc<Entry>,
    stream: &mut Option<(WriteSide, Arc<StreamLife>)>,
    queued: &Queued,
) -> io::Result<()> {
    let mut bytes = Vec::new();
    if stream.is_none() {
        // A command receives with a large window from its first byte; the
        // connection's stream is the peer's allowance beside its streams.
        let window = (lane.kind == LaneKind::Command).then_some(WINDOW_COMMAND);
        let counted = lane.kind != LaneKind::Connection;
        let (opened, permit) = shared.open_stream(window, counted).await?;
        let stream_life = StreamLife::opened(permit);
        let (read, write) = tokio::io::split(opened.compat());
        let read = EndingRead::new(read, shared);
        shared.readers.spawn(read_stream(
            shared.clone(),
            read,
            Some((life.clone(), entry.peer_done.clone())),
            stream_life.clone(),
        ));
        bytes = framing::lane_header(lane, false);
        *stream = Some((write, stream_life));
    }
    let (write, _) = stream.as_mut().expect("the stream was opened above");
    let joined = queued.message.len() <= JOINED_BYTES;
    bytes.extend_from_slice(&framing::frame_header(queued.message.len()));
    if joined {
        bytes.extend_from_slice(&queued.message);
    }
    tokio::select! {
        () = shared.closed.cancelled() => Err(io::ErrorKind::BrokenPipe.into()),
        written = async {
            write.write_all(&bytes).await?;
            if !joined {
                write.write_all(&queued.message).await?;
            }
            Ok(())
        } => written,
    }
}

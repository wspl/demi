//! A pipe between two devices on the pair's direct channel
//! (`crates/runner-protocol/src/wire/pipe_frames.rs`): the source moves its
//! bytes there once the pair's peer is connected, and back to the relay when
//! the channel goes; the sink follows each move. No byte is lost or repeated.
//!
//! ```text
//! source                          relay                        sink
//!   0..N        ------------------------------------------->   0..N
//!   move{direct, at: N}  ------------------------------------>  waits for the
//!   N..M        ========== direct channel ==================>  channel
//!   channel gone; G = what the sink granted back
//!   move{relay, at: G}   ------------------------------------>  drops G..M
//!   G..         ------------------------------------------->   G..
//! ```
//!
//! The source keeps what it sent on the channel until the sink grants it back,
//! which the sink does once its reader took it, so the source holds at most
//! the channel's window. A pipe that ends on the channel waits for the sink to
//! grant everything, moves back to the relay at its last byte, and ends there,
//! so the relay always carries the end.
//!
//! The pairs are the runner's ([`PipePairs`]): this crate knows no peer, only
//! channels that carry bytes.

use std::collections::VecDeque;
use std::io;

use bytes::Bytes;
use demi_runner_protocol::wire::PipePath;
use futures_util::future::BoxFuture;
use futures_util::stream::BoxStream;
use futures_util::StreamExt as _;
use tokio::sync::watch;

/// The runner's pairs with the other runners of the user, as pipes use them
/// (`crates/runner-direct/src/pairs.rs`).
pub trait PipePairs: Send + Sync + 'static {
    /// A pipe's place among the connections of this runner's pair with the
    /// runner of `device`, which starts on the relay. It keeps the pair's
    /// peer up, and has a failed one tried again, while it lives.
    fn relay(&self, device: String) -> BoxFuture<'static, Box<dyn PairPlace>>;
    /// The channel the runner of `device` opens for pipe `pipe`'s bytes from
    /// `at` on, whose sink this runner is, once it arrives; none once this
    /// runner's pairs ended. A channel from an earlier move, which its source
    /// left, is not it.
    fn accept(&self, device: String, pipe: String, at: u64) -> BoxFuture<'static, Option<Box<dyn PipeChannel>>>;
}

/// The pairs of the runner's connection to the backend now, none between
/// two connections.
pub type CurrentPairs = watch::Receiver<Option<std::sync::Arc<dyn PipePairs>>>;

/// A pipe's place among its pair's connections.
pub trait PairPlace: Send + Sync {
    /// Resolves once the pair's peer is connected and has room for the pipe,
    /// which then counts as direct; false once the pairs ended. The source
    /// keeps the future until it resolves: one dropped after the peer had room
    /// would leave the pipe counted direct on the relay.
    fn ready(&self) -> BoxFuture<'static, bool>;
    /// Opens a channel for pipe `pipe`'s bytes from `at` on, on the peer
    /// [`PairPlace::ready`] found; none when it could not, which puts the
    /// pipe back on the relay.
    fn open(&self, pipe: String, at: u64) -> BoxFuture<'_, Option<Box<dyn PipeChannel>>>;
    /// The pipe's channel went: it is on the relay.
    fn relayed(&self);
}

/// A channel of a pair's peer that carries a pipe's bytes.
pub trait PipeChannel: Send + Sync {
    /// The pipe's byte its first byte is, as its header says.
    fn at(&self) -> u64;
    /// Sends `bytes` as the channel's window lets it; false once it went.
    fn send(&self, bytes: Bytes) -> BoxFuture<'_, bool>;
    /// Says that the source sent everything; false once it went.
    fn end(&self) -> BoxFuture<'_, bool>;
    /// The bytes the sink took, from the channel's first.
    fn granted(&self) -> watch::Receiver<u64>;
    /// Resolves once the channel went.
    fn gone(&self) -> BoxFuture<'_, ()>;
    /// The source's next bytes, or its end; none once the channel went.
    fn receive(&mut self) -> BoxFuture<'_, Option<Carried>>;
    /// The sink's reader took `bytes`, which the channel grants back.
    fn took(&self, bytes: usize);
}

/// What a pipe's channel carries.
#[derive(Debug, PartialEq, Eq)]
pub enum Carried {
    Bytes(Bytes),
    /// The source sent everything it sends on the channel.
    End,
}

/// What a pipe's relay leg carries to its sink, besides its end.
#[derive(Debug, PartialEq, Eq)]
pub enum Relayed {
    Bytes(Bytes),
    /// The bytes from `at` on come over `to`; `source` is the source's
    /// device, as the backend names it.
    Moved { to: PipePath, at: u64, source: String },
}

/// The bytes the source keeps on the channel until the sink grants them.
struct Unconfirmed {
    /// The pipe's byte the channel's first is.
    base: u64,
    /// The pipe's byte the first kept one is.
    from: u64,
    kept: VecDeque<Bytes>,
}

impl Unconfirmed {
    fn new(base: u64) -> Self {
        Self {
            base,
            from: base,
            kept: VecDeque::new(),
        }
    }

    fn keep(&mut self, bytes: Bytes) {
        self.kept.push_back(bytes);
    }

    /// The sink granted `granted` of the channel's bytes: those go.
    fn confirm(&mut self, granted: u64) {
        let confirmed = self.base + granted;
        while let Some(front) = self.kept.front_mut() {
            let end = self.from + front.len() as u64;
            if end <= confirmed {
                self.from = end;
                self.kept.pop_front();
            } else {
                if confirmed > self.from {
                    let cut = (confirmed - self.from) as usize;
                    *front = front.slice(cut..);
                    self.from = confirmed;
                }
                break;
            }
        }
    }

    /// Where the pipe moves back to the relay, and the bytes it sends there
    /// again.
    fn take(self) -> (u64, VecDeque<Bytes>) {
        (self.from, self.kept)
    }
}

/// A pipe's source on its direct channel.
pub(crate) struct DirectSource {
    pub(crate) channel: Box<dyn PipeChannel>,
    granted: watch::Receiver<u64>,
    unconfirmed: Unconfirmed,
}

/// Why a source left its channel.
pub(crate) struct LeftChannel {
    /// The pipe's byte it moves back to the relay at.
    pub(crate) at: u64,
    /// What it sends there again, from `at` on.
    pub(crate) again: VecDeque<Bytes>,
}

impl DirectSource {
    /// The source's bytes from `at` on go over `channel`.
    pub(crate) fn new(channel: Box<dyn PipeChannel>, at: u64) -> Self {
        let granted = channel.granted();
        Self {
            channel,
            granted,
            unconfirmed: Unconfirmed::new(at),
        }
    }

    /// Sends `bytes` on the channel; the channel's end when it went.
    pub(crate) async fn send(&mut self, bytes: Bytes) -> Result<(), LeftChannel> {
        self.unconfirmed.keep(bytes.clone());
        let sent = tokio::select! {
            sent = self.channel.send(bytes) => sent,
            () = self.channel.gone() => false,
        };
        if sent { Ok(()) } else { Err(self.leave()) }
    }

    /// Resolves when the sink granted more, or with the channel's end when it
    /// went.
    pub(crate) async fn granted(&mut self) -> Result<(), LeftChannel> {
        let changed = tokio::select! {
            changed = self.granted.changed() => changed.is_ok(),
            () = self.channel.gone() => false,
        };
        if changed {
            let granted = *self.granted.borrow_and_update();
            self.unconfirmed.confirm(granted);
            Ok(())
        } else {
            Err(self.leave())
        }
    }

    /// Ends the channel once the source sent everything, and waits until the
    /// sink granted it all: then the pipe moves back to the relay at its last
    /// byte, with nothing to send again.
    pub(crate) async fn finish(mut self) -> LeftChannel {
        let ended = tokio::select! {
            ended = self.channel.end() => ended,
            () = self.channel.gone() => false,
        };
        if !ended {
            return self.leave();
        }
        while !self.unconfirmed.kept.is_empty() {
            if let Err(left) = self.granted().await {
                return left;
            }
        }
        self.leave()
    }

    /// Leaves the channel at the last byte the sink granted.
    fn leave(&mut self) -> LeftChannel {
        let granted = *self.granted.borrow_and_update();
        self.unconfirmed.confirm(granted);
        let unconfirmed = std::mem::replace(&mut self.unconfirmed, Unconfirmed::new(0));
        let (at, again) = unconfirmed.take();
        LeftChannel { at, again }
    }
}

/// Where a sink reads a pipe's bytes from.
enum Following {
    /// The relay, dropping the first `skip` bytes, which it holds already.
    Relay { skip: u64 },
    /// The relay said the bytes go direct: the source's channel is on its way.
    Awaiting(BoxFuture<'static, Option<Box<dyn PipeChannel>>>),
    Direct(Box<dyn PipeChannel>),
    /// The channel ended or went: the relay says where the pipe goes on.
    Left,
}

/// The sink of pipe `pipe`: its bytes, in order, once each, from its relay
/// leg `relay` and the direct channels its moves name, which `pairs` brings.
pub(crate) fn follow(
    pipe: String,
    relay: BoxStream<'static, io::Result<Relayed>>,
    pairs: CurrentPairs,
) -> BoxStream<'static, io::Result<Bytes>> {
    struct Sink {
        pipe: String,
        relay: BoxStream<'static, io::Result<Relayed>>,
        /// The pairs a move's channel arrives on: those of the runner's
        /// connection at the move.
        pairs: CurrentPairs,
        following: Following,
        /// The pipe's bytes the sink holds.
        at: u64,
        over: bool,
    }

    impl Sink {
        /// The next bytes, none at the pipe's end.
        async fn next(&mut self) -> Option<io::Result<Bytes>> {
            /// What the sink heard while it followed.
            enum Heard {
                Relay(Option<io::Result<Relayed>>),
                Accepted(Option<Box<dyn PipeChannel>>),
                Carried(Option<Carried>),
            }
            loop {
                if self.over {
                    return None;
                }
                let heard = match &mut self.following {
                    Following::Relay { .. } | Following::Left => Heard::Relay(self.relay.next().await),
                    Following::Awaiting(accepting) => tokio::select! {
                        channel = accepting => Heard::Accepted(channel),
                        item = self.relay.next() => Heard::Relay(item),
                    },
                    Following::Direct(channel) => tokio::select! {
                        biased;
                        carried = channel.receive() => Heard::Carried(carried),
                        item = self.relay.next() => Heard::Relay(item),
                    },
                };
                match heard {
                    Heard::Relay(item) => {
                        if let Following::Relay { skip } = &mut self.following {
                            let item = match item {
                                Some(Ok(item)) => item,
                                Some(Err(error)) => return self.fail(error),
                                None => {
                                    self.over = true;
                                    return None;
                                }
                            };
                            match item {
                                Relayed::Bytes(mut bytes) => {
                                    if *skip > 0 {
                                        let dropped = (*skip).min(bytes.len() as u64);
                                        bytes = bytes.slice(dropped as usize..);
                                        *skip -= dropped;
                                    }
                                    if bytes.is_empty() {
                                        continue;
                                    }
                                    self.at += bytes.len() as u64;
                                    return Some(Ok(bytes));
                                }
                                Relayed::Moved { to, at, source } => {
                                    if let Err(error) = self.moved(to, at, source) {
                                        return self.fail(error);
                                    }
                                }
                            }
                        } else if let Some(error) = self.on_relay(item) {
                            return self.fail(error);
                        }
                    }
                    Heard::Accepted(None) => {
                        // The pairs ended: the source's channel went with
                        // them, and the relay says where the bytes go on.
                        self.following = Following::Left;
                    }
                    Heard::Accepted(Some(channel)) => {
                        if channel.at() != self.at {
                            return self.fail(invalid(format!(
                                "the direct channel starts at byte {}, where the pipe is at {}",
                                channel.at(),
                                self.at
                            )));
                        }
                        self.following = Following::Direct(channel);
                    }
                    Heard::Carried(Some(Carried::Bytes(bytes))) => {
                        if let Following::Direct(channel) = &self.following {
                            channel.took(bytes.len());
                        }
                        self.at += bytes.len() as u64;
                        return Some(Ok(bytes));
                    }
                    Heard::Carried(Some(Carried::End) | None) => self.following = Following::Left,
                }
            }
        }

        /// What the relay says while the bytes go direct: only a move back.
        fn on_relay(&mut self, item: Option<io::Result<Relayed>>) -> Option<io::Error> {
            match item {
                Some(Ok(Relayed::Moved { to, at, source })) => self.moved(to, at, source).err(),
                Some(Ok(Relayed::Bytes(_))) => Some(invalid("the relay carried bytes while they went direct".into())),
                Some(Err(error)) => Some(error),
                None => Some(invalid("the pipe ended while its bytes went direct".into())),
            }
        }

        fn moved(&mut self, to: PipePath, at: u64, source: String) -> io::Result<()> {
            match to {
                PipePath::Relay => {
                    if at > self.at {
                        return Err(invalid(format!(
                            "the pipe moved back at byte {at}, past the {} it holds",
                            self.at
                        )));
                    }
                    self.following = Following::Relay { skip: self.at - at };
                }
                PipePath::Direct => {
                    if at != self.at {
                        return Err(invalid(format!(
                            "the pipe moved direct at byte {at}, where it is at {}",
                            self.at
                        )));
                    }
                    let pairs = self.pairs.borrow().clone();
                    self.following = match pairs {
                        Some(pairs) => Following::Awaiting(pairs.accept(source, self.pipe.clone(), at)),
                        // No pairs, no channel: the source finds its
                        // channel gone and moves back.
                        None => Following::Left,
                    };
                }
            }
            Ok(())
        }

        fn fail(&mut self, error: io::Error) -> Option<io::Result<Bytes>> {
            self.over = true;
            Some(Err(error))
        }
    }

    let sink = Sink {
        pipe,
        relay,
        pairs,
        following: Following::Relay { skip: 0 },
        at: 0,
        over: false,
    };
    futures_util::stream::unfold(sink, |mut sink| async move {
        sink.next().await.map(|item| (item, sink))
    })
    .boxed()
}

fn invalid(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

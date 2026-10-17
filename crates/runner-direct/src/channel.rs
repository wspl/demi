//! A data channel's reader and writer, the one flow control every channel
//! of a peer uses, a page's file operation's and a forwarded TCP
//! connection's alike (`direct-channel.md` § Operations on the channel,
//! `port-forwarding.md` § Flow control).
//!
//! What goes out: the channel takes its producer's next message only while
//! its queue holds less than [`QUEUE_BYTES`], so a reader paces what it
//! reads. A credited channel, a forwarded connection's, also sends a binary
//! message only within the window the other side granted, which starts at
//! [`QUEUE_BYTES`] and grows by each `{ grant }` it receives.
//!
//! What comes in goes to the channel's consumer as it arrives; a credited
//! channel grants the other side what its consumer took, once the consumer
//! says it took it, so nothing piles up in the runner.

use bytes::Bytes;
use demi_runner_protocol::direct::QUEUE_BYTES;
use demi_runner_protocol::forward::Grant;
use str0m::Rtc;
use str0m::channel::ChannelId;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::operation::{Input, Output};

/// How many bytes a consumer takes before its channel grants them back in
/// one message.
const GRANT_EVERY: usize = QUEUE_BYTES / 4;

/// What a channel's consumer takes after the first message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Bytes, then `{ end: true }`.
    Write,
    /// Bytes only.
    Stream,
    /// `paths` messages only.
    Watch,
    /// Nothing more.
    Request,
    /// A forwarded connection's bytes, its end as `{ end: true }`, and the
    /// other side's grants.
    Forward,
    /// Every message as it comes: a test's page.
    Raw,
}

/// Where a channel is.
pub(crate) enum Phase {
    /// The other side opened it; its first message, the header, decides
    /// what it carries.
    Header,
    /// This side opened it, and it is not open yet.
    Opening { kind: Kind, input: mpsc::UnboundedSender<Input> },
    /// It carries its operation; the other side's messages go to its
    /// consumer.
    Running { kind: Kind, input: mpsc::UnboundedSender<Input> },
    /// Its operation ended, or it was refused: it closes once its messages
    /// are out.
    Closing,
}

/// The window of a credited channel.
pub(crate) struct Credit {
    /// The bytes this side may still send.
    window: usize,
    /// The bytes this side's consumer took and this side has not granted
    /// back yet.
    owed: usize,
}

impl Credit {
    pub(crate) fn new() -> Self {
        Self {
            window: QUEUE_BYTES,
            owed: 0,
        }
    }
}

/// One channel, from its open to its close.
pub(crate) struct Channel {
    pub(crate) phase: Phase,
    /// The message for the other side that waits for room.
    pending: Option<Output>,
    /// The producer's messages; none once it ended, or for a channel
    /// refused at once.
    output: Option<mpsc::Receiver<Output>>,
    /// The channel's queue, and its window, have room for another of the
    /// producer's messages.
    room: bool,
    /// Ends the operation: cancelled when the channel goes.
    pub(crate) cancel: CancellationToken,
    credit: Option<Credit>,
    /// Cancelled by a forwarded connection that ended before its bytes all
    /// went: the channel closes at once, what waits for room or for the
    /// other side's window dropped, rather than once it drained.
    abort: Option<(CancellationToken, std::pin::Pin<Box<tokio_util::sync::WaitForCancellationFutureOwned>>)>,
    aborted: bool,
}

impl Drop for Channel {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

/// What a channel's messages did, which str0m then has to send.
pub(crate) struct Pumped {
    pub(crate) wrote: bool,
    /// The channel is done: closed, or to be closed.
    pub(crate) done: bool,
}

impl Channel {
    /// A channel the other side opened, waiting for its header.
    pub(crate) fn awaiting_header() -> Self {
        Self {
            phase: Phase::Header,
            pending: None,
            output: None,
            room: false,
            cancel: CancellationToken::new(),
            credit: None,
            abort: None,
            aborted: false,
        }
    }

    /// A channel that sends `message` and closes.
    pub(crate) fn closing(message: Output) -> Self {
        Self {
            phase: Phase::Closing,
            pending: Some(message),
            output: None,
            room: false,
            cancel: CancellationToken::new(),
            credit: None,
            abort: None,
            aborted: false,
        }
    }

    /// A channel this side opens, of `kind`, whose first message is
    /// `first`, if any; credited when `credit`.
    pub(crate) fn opening(
        kind: Kind,
        first: Option<Output>,
        input: mpsc::UnboundedSender<Input>,
        output: mpsc::Receiver<Output>,
        cancel: CancellationToken,
        credit: bool,
    ) -> Self {
        Self {
            phase: Phase::Opening { kind, input },
            pending: first,
            output: Some(output),
            room: false,
            cancel,
            credit: credit.then(Credit::new),
            abort: None,
            aborted: false,
        }
    }

    /// Closes the channel at once when `abort` is cancelled, whatever it
    /// still holds.
    pub(crate) fn abort_on(&mut self, abort: CancellationToken) {
        let waiting = Box::pin(abort.clone().cancelled_owned());
        self.abort = Some((abort, waiting));
    }

    /// Drops what the channel holds and closes it.
    fn abort_now(&mut self) {
        self.abort = None;
        self.aborted = true;
        self.pending = None;
        self.output = None;
        self.room = false;
        self.phase = Phase::Closing;
    }

    /// Whether its abort was asked for, which it takes at once.
    fn take_abort(&mut self, context: Option<&mut std::task::Context<'_>>) -> bool {
        use std::future::Future as _;
        let Some((abort, waiting)) = &mut self.abort else {
            return self.aborted;
        };
        // Polled, the wait wakes the peer when the abort comes.
        let asked = match context {
            Some(context) => waiting.as_mut().poll(context).is_ready(),
            None => abort.is_cancelled(),
        };
        if asked {
            self.abort_now();
        }
        self.aborted
    }

    /// The header arrived: the channel carries an operation of `kind`,
    /// whose consumer is `input` and producer `output`.
    pub(crate) fn run(
        &mut self,
        kind: Kind,
        input: mpsc::UnboundedSender<Input>,
        output: mpsc::Receiver<Output>,
        credit: bool,
    ) {
        self.phase = Phase::Running { kind, input };
        self.output = Some(output);
        self.room = true;
        self.credit = credit.then(Credit::new);
    }

    /// The channel this side opened is open.
    pub(crate) fn opened(&mut self) {
        if let Phase::Opening { kind, input } =
            std::mem::replace(&mut self.phase, Phase::Closing)
        {
            self.phase = Phase::Running { kind, input };
            self.room = self.pending.is_none();
        }
    }

    /// Its consumer took `bytes` of what the other side sent.
    pub(crate) fn took(&mut self, bytes: usize) {
        if let Some(credit) = &mut self.credit {
            credit.owed += bytes;
        }
    }

    /// One message of the other side's after the header: false when the
    /// channel's consumer cannot read it, which ends the channel.
    pub(crate) fn receive(&mut self, binary: bool, data: Vec<u8>) -> bool {
        let Phase::Running { kind, input } = &self.phase else {
            return true;
        };
        let message = match (kind, binary) {
            (Kind::Raw, false) => match String::from_utf8(data) {
                Ok(text) => Some(Input::Text(text)),
                Err(_) => None,
            },
            (Kind::Raw | Kind::Forward | Kind::Write | Kind::Stream, true) => {
                Some(Input::Bytes(Bytes::from(data)))
            }
            (Kind::Forward, false) => {
                if let (Some(credit), Ok(grant)) =
                    (&mut self.credit, serde_json::from_slice::<Grant>(&data))
                {
                    credit.window += grant.grant as usize;
                    return true;
                }
                crate::operation::text_input(*kind, &data)
            }
            (_, false) => crate::operation::text_input(*kind, &data),
            (Kind::Watch | Kind::Request, true) => None,
        };
        match message {
            Some(message) => {
                // A consumer that ended takes nothing more.
                let _ = input.send(message);
                true
            }
            None => {
                self.cancel.cancel();
                self.output = None;
                self.phase = Phase::Closing;
                false
            }
        }
    }

    /// Writes the channel's waiting messages while its queue and window have
    /// room, its grants first; a channel whose producer ended is done once
    /// its queue is empty.
    pub(crate) fn pump(&mut self, rtc: &mut Rtc, id: ChannelId) -> Pumped {
        if self.take_abort(None) {
            return Pumped {
                wrote: false,
                done: true,
            };
        }
        if matches!(self.phase, Phase::Opening { .. }) {
            return Pumped {
                wrote: false,
                done: false,
            };
        }
        let Some(mut sending) = rtc.channel(id) else {
            return Pumped {
                wrote: false,
                done: true,
            };
        };
        let mut wrote = false;
        if let Some(credit) = &mut self.credit
            && credit.owed >= GRANT_EVERY.min(QUEUE_BYTES)
        {
            let grant = Grant {
                grant: u32::try_from(credit.owed).unwrap_or(u32::MAX),
            };
            let text = serde_json::to_string(&grant).expect("a grant serializes");
            match sending.write(false, text.as_bytes()) {
                Ok(true) => {
                    credit.owed -= grant.grant as usize;
                    wrote = true;
                }
                // Every channel's queue together is full: the grant goes
                // with the next pump.
                Ok(false) => {}
                Err(error) => {
                    tracing::debug!("a direct channel failed: {error}");
                    return Pumped { wrote, done: true };
                }
            }
        }
        loop {
            if self.pending.is_none()
                && let Some(output) = &mut self.output
            {
                match output.try_recv() {
                    Ok(message) => self.pending = Some(message),
                    Err(mpsc::error::TryRecvError::Empty) => {}
                    Err(mpsc::error::TryRecvError::Disconnected) => {
                        self.output = None;
                        self.phase = Phase::Closing;
                    }
                }
            }
            let Some(message) = &self.pending else {
                break;
            };
            let queued = sending.buffered_amount();
            if queued > 0 && queued + message.len() > QUEUE_BYTES {
                break;
            }
            let binary = matches!(message, Output::Binary(_));
            if binary
                && let Some(credit) = &self.credit
                && message.len() > credit.window
            {
                break;
            }
            let written = match message {
                Output::Text(text) => sending.write(false, text.as_bytes()),
                Output::Binary(bytes) => sending.write(true, bytes),
            };
            match written {
                Ok(true) => {
                    if binary && let Some(credit) = &mut self.credit {
                        credit.window -= message.len();
                    }
                    self.pending = None;
                    wrote = true;
                }
                // Every channel's queue together is full.
                Ok(false) => break,
                Err(error) => {
                    tracing::debug!("a direct channel failed: {error}");
                    return Pumped { wrote, done: true };
                }
            }
        }
        let windowed = self
            .credit
            .as_ref()
            .is_none_or(|credit| credit.window >= demi_runner_protocol::direct::MESSAGE_BYTES);
        self.room = self.pending.is_none()
            && self.output.is_some()
            && windowed
            && sending.buffered_amount() < QUEUE_BYTES;
        let done = matches!(self.phase, Phase::Closing)
            && self.pending.is_none()
            && self.output.is_none()
            && sending.buffered_amount() == 0;
        Pumped { wrote, done }
    }

    /// Takes the producer's next message once the channel has room, or
    /// learns that the producer ended; ready when it did either.
    pub(crate) fn poll_output(&mut self, context: &mut std::task::Context<'_>) -> std::task::Poll<()> {
        use std::task::Poll;
        if !self.aborted && self.take_abort(Some(context)) {
            return Poll::Ready(());
        }
        if !self.room {
            return Poll::Pending;
        }
        let Some(output) = &mut self.output else {
            return Poll::Pending;
        };
        match output.poll_recv(context) {
            Poll::Ready(Some(message)) => {
                self.pending = Some(message);
                Poll::Ready(())
            }
            Poll::Ready(None) => {
                self.output = None;
                self.phase = Phase::Closing;
                Poll::Ready(())
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

//! The root's request output, as the pages see it beside the chips row
//! (`docs/decisions/0012-live-output-tokens.md`).
//!
//! The root's session counts the output of the request it runs and announces
//! each change. The tree sends the pages a request's start, its end and the
//! turn's end at once, and the counting in between at most every
//! [`INTERVAL`]: the latest reading, its streaming time grown by how long it
//! waited, so the time is the time it leaves. An open's handshake carries the
//! reading as the session has it.
//!
//! The timer runs only while a change waits. The sending task ends with the
//! tree, which aborts it, and holds nothing of the session, so a closed
//! tree's session goes with the tree.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

use demi_conversation_socket_protocol::{RequestOutput, ServerFrame};
use tokio::{sync::Notify, time::Instant};

use super::FrameSink;

/// How often the reading of a streaming request goes, at most.
pub(crate) const INTERVAL: Duration = Duration::from_millis(500);

/// The root's reading, and what of it the pages were sent.
pub(crate) struct OutputReading {
    sink: Rc<FrameSink>,
    /// The request of the last frame sent and whether it had ended; none
    /// when that frame had no reading.
    sent: RefCell<Option<(String, bool)>>,
    /// When the last frame went.
    sent_at: Cell<Option<Instant>>,
    /// The change of the streaming request that waits for the next send,
    /// and when the session announced it.
    waiting: RefCell<Option<(RequestOutput, Instant)>>,
    /// Wakes the sending task when a change starts to wait.
    wake: Notify,
}

impl OutputReading {
    pub(crate) fn new(sink: Rc<FrameSink>) -> Rc<Self> {
        Rc::new(Self {
            sink,
            sent: RefCell::new(None),
            sent_at: Cell::new(None),
            waiting: RefCell::new(None),
            wake: Notify::new(),
        })
    }

    /// The session announced `output`: another request, its end or the
    /// turn's goes at once, more of the same request with the next send.
    pub(crate) fn changed(&self, output: &Option<RequestOutput>) {
        match output {
            Some(output) if *self.sent.borrow() == key(Some(output)) => {
                if self.waiting.replace(Some((output.clone(), Instant::now()))).is_none() {
                    self.wake.notify_one();
                }
            }
            _ => {
                self.waiting.replace(None);
                self.send(output.clone());
            }
        }
    }

    /// When the waiting change is due: at once after a quiet interval.
    fn due(&self) -> Option<Instant> {
        self.waiting.borrow().as_ref()?;
        Some(
            self.sent_at
                .get()
                .map_or_else(Instant::now, |sent| sent + INTERVAL),
        )
    }

    /// Sends the waiting change, its streaming time as it is now.
    fn send_waiting(&self) {
        let Some((mut output, announced)) = self.waiting.take() else {
            return;
        };
        let waited = u64::try_from(announced.elapsed().as_millis()).unwrap_or(u64::MAX);
        output.streamed_ms = output.streamed_ms.map(|streamed| streamed.saturating_add(waited));
        self.send(Some(output));
    }

    fn send(&self, output: Option<RequestOutput>) {
        self.sent.replace(key(output.as_ref()));
        self.sent_at.set(Some(Instant::now()));
        self.sink.emit(ServerFrame::RequestOutput { output });
    }
}

fn key(output: Option<&RequestOutput>) -> Option<(String, bool)> {
    output.map(|output| (output.request_id.clone(), output.ended))
}

/// The tree's task that sends the waiting change of the root's reading,
/// [`INTERVAL`] after the last frame.
pub(crate) async fn send_changes(reading: Rc<OutputReading>) {
    loop {
        // A frame that went at once meanwhile moves the time the waiting
        // change is due, so it is read again after each sleep.
        match reading.due() {
            None => reading.wake.notified().await,
            Some(due) if due > Instant::now() => tokio::time::sleep_until(due).await,
            Some(_) => reading.send_waiting(),
        }
    }
}

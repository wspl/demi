//! The starter lane of a program's core
//! (`crates/web-api-protocol/src/starter.rs`): the program that opened the
//! core opens it on its page connection, and the core's starter sends its
//! notifications on it and waits for the program's answers.
//!
//! One lane is open at a time. A notification sent while none is, or whose
//! lane ends before the program answers, reached no one.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use axum::extract::ws::close_code;
use demi_core::starter::Notified;
use demi_core::{Lane, LaneClose, LaneEnd, LaneMessage, LaneReader};
use demi_web_api_protocol::push::PushMessage;
use demi_web_api_protocol::starter::{StarterAnswer, StarterRequest};
use tokio::sync::{mpsc, oneshot};

/// One core's starter lane, open or not.
#[derive(Default)]
pub struct StarterLane(Mutex<Opens>);

#[derive(Default)]
struct Opens {
    /// How many lanes were opened for the core.
    count: u64,
    /// The open lane's notifications to send, with the lane's number among
    /// them; none while no lane is open.
    open: Option<(u64, mpsc::UnboundedSender<Notification>)>,
}

/// A notification to send, and where its answer goes.
struct Notification {
    message: PushMessage,
    answer: oneshot::Sender<Notified>,
}

impl StarterLane {
    fn opens(&self) -> std::sync::MutexGuard<'_, Opens> {
        // No section panics while it holds the lock.
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether a lane is open.
    pub fn is_open(&self) -> bool {
        self.opens().open.is_some()
    }

    /// Sends `message` on the open lane and answers how the program says
    /// it went; nowhere while no lane is open, or when the lane ends first.
    pub async fn notify(&self, message: PushMessage) -> Notified {
        let (answer, answered) = oneshot::channel();
        let sent = match &self.opens().open {
            Some((_, notifications)) => notifications.send(Notification { message, answer }).is_ok(),
            None => false,
        };
        if !sent {
            return Notified::Nowhere;
        }
        answered.await.unwrap_or(Notified::Nowhere)
    }

    /// Ends the open lane, as the core closes.
    pub fn end(&self) {
        self.opens().open.take();
    }

    /// Takes the lane for an open; none while one is open.
    pub(crate) fn claim(self: &Arc<Self>) -> Option<Claim> {
        let mut opens = self.opens();
        if opens.open.is_some() {
            return None;
        }
        opens.count += 1;
        let (notifications, received) = mpsc::unbounded_channel();
        opens.open = Some((opens.count, notifications));
        Some(Claim {
            lane: self.clone(),
            number: opens.count,
            notifications: received,
        })
    }
}

/// An open of the starter lane, from its claim until it ends.
pub(crate) struct Claim {
    lane: Arc<StarterLane>,
    number: u64,
    notifications: mpsc::UnboundedReceiver<Notification>,
}

impl Claim {
    /// Sends the core's notifications on `lane` and hands the program's
    /// answers back, until the program closes the lane, its connection
    /// ends, or the core closes.
    pub(crate) async fn serve(mut self, mut lane: Lane, mut reader: LaneReader) {
        let mut waiting: HashMap<u32, oneshot::Sender<Notified>> = HashMap::new();
        let mut next = 0u32;
        let end = loop {
            tokio::select! {
                notification = self.notifications.recv() => {
                    // The core closes: its starter ended the lane.
                    let Some(Notification { message, answer }) = notification else {
                        break LaneEnd::Close(LaneClose { code: close_code::AWAY, reason: "core_closing" });
                    };
                    next = next.wrapping_add(1);
                    let request = StarterRequest::Notify { id: next, message };
                    let text = serde_json::to_string(&request).expect("a starter request serializes");
                    if lane.send(text).await.is_err() {
                        break LaneEnd::Gone;
                    }
                    waiting.insert(next, answer);
                }
                message = reader.next() => {
                    let Some(message) = message else {
                        break LaneEnd::Gone;
                    };
                    let answer = match message {
                        LaneMessage::Text(text) => serde_json::from_str::<StarterAnswer>(&text).ok(),
                        LaneMessage::Binary(_) => None,
                    };
                    let (id, outcome) = match answer {
                        Some(StarterAnswer::Sending { id, to }) => (id, Notified::Sending(to as usize)),
                        Some(StarterAnswer::Shown { id }) => (id, Notified::Shown),
                        Some(StarterAnswer::Nowhere { id }) => (id, Notified::Nowhere),
                        None => {
                            break LaneEnd::Close(LaneClose { code: close_code::POLICY, reason: "unexpected_message" });
                        }
                    };
                    if let Some(answer) = waiting.remove(&id) {
                        // A notification whose sender left needs no answer.
                        let _ = answer.send(outcome);
                    }
                }
            }
        };
        // The notifications still waiting reached no one: their senders hear
        // so as their answers drop.
        drop(waiting);
        lane.end(reader, end);
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        let mut opens = self.lane.opens();
        // A lane opened since this one ended is the next one's.
        if opens.open.as_ref().is_some_and(|(number, _)| *number == self.number) {
            opens.open = None;
        }
    }
}

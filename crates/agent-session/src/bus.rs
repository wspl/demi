//! Delivery of a session's events.
//!
//! Each event, once its change is complete, reaches every listener in the order
//! the changes happened. A listener may call back into the session; what that
//! causes is delivered after the current event has reached every listener.
//! Events may be held and then delivered together, such as a turn's end.
//!
//! A hold keeps the transcript and the phase together. What is part of
//! neither passes it, delivered in its order among the events not held:
//!
//! - the live reading of a request's output (`SessionEvent::OutputChanged`);
//! - a change of the waiting inputs (`SessionEvent::PendingSteersChanged`,
//!   `SessionEvent::QueueChanged`) made apart from the transcript and the
//!   phase, such as a steer, its Recall or a message queued. A client is told
//!   such a command's answer at once, and must see the list it changed with
//!   it, however long the turn's end waits. A change made together with the
//!   transcript, such as a steer written into it, stays with it, and so does
//!   one whose earlier state is held, which would otherwise arrive after it.

use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
};

use super::SessionEvent;

type Listener = Box<dyn FnMut(&SessionEvent)>;

struct Slot {
    id: u64,
    /// Taken out while it runs, so that it may call back into the session.
    listener: Option<Listener>,
    removed: bool,
}

#[derive(Default)]
pub(crate) struct EventBus {
    slots: RefCell<Vec<Slot>>,
    queue: RefCell<VecDeque<SessionEvent>>,
    /// The events a hold lets pass, delivered before the queue's.
    passing: RefCell<VecDeque<SessionEvent>>,
    delivering: Cell<bool>,
    /// What `deliver` is given waits in the queue until `release`.
    held: Cell<bool>,
    next_id: Cell<u64>,
}

impl EventBus {
    pub(crate) fn subscribe(&self, listener: Listener) -> u64 {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        self.slots.borrow_mut().push(Slot {
            id,
            listener: Some(listener),
            removed: false,
        });
        id
    }

    pub(crate) fn unsubscribe(&self, id: u64) {
        let mut slots = self.slots.borrow_mut();
        if let Some(slot) = slots.iter_mut().find(|slot| slot.id == id) {
            slot.removed = true;
            slot.listener = None;
        }
        if !self.delivering.get() {
            slots.retain(|slot| !slot.removed);
        }
    }

    /// Delivers `events` after those already waiting. A call made while a
    /// delivery runs only queues its events: the running delivery delivers
    /// them in order. While the bus holds, they wait, except those a hold
    /// lets pass.
    pub(crate) fn deliver(&self, events: Vec<SessionEvent>) {
        if events.is_empty() {
            return;
        }
        let with_transcript = events.iter().any(|event| {
            matches!(event, SessionEvent::TranscriptChanged { .. } | SessionEvent::PhaseChanged { .. })
        });
        for event in events {
            if self.held.get() && self.passes(&event, with_transcript) {
                self.passing.borrow_mut().push_back(event);
            } else {
                self.queue.borrow_mut().push_back(event);
            }
        }
        self.drain();
    }

    /// Whether a hold lets `event` pass, given whether its change also
    /// changed the transcript or the phase.
    fn passes(&self, event: &SessionEvent, with_transcript: bool) -> bool {
        match event {
            SessionEvent::OutputChanged { .. } => true,
            SessionEvent::PendingSteersChanged { .. } | SessionEvent::QueueChanged { .. } => {
                let kind = std::mem::discriminant(event);
                !with_transcript && !self.queue.borrow().iter().any(|held| std::mem::discriminant(held) == kind)
            }
            _ => false,
        }
    }

    /// Keeps what `deliver` is given from now on until `release`. Called
    /// outside a delivery, so nothing given before it waits.
    pub(crate) fn hold(&self) {
        debug_assert!(!self.delivering.get(), "a hold starts outside a delivery");
        self.held.set(true);
    }

    /// Delivers what the bus held, in order, and every later event at once.
    pub(crate) fn release(&self) {
        if self.held.replace(false) {
            self.drain();
        }
    }

    fn drain(&self) {
        if self.delivering.replace(true) {
            return;
        }
        loop {
            let passing = self.passing.borrow_mut().pop_front();
            let next = passing.or_else(|| {
                if self.held.get() {
                    None
                } else {
                    self.queue.borrow_mut().pop_front()
                }
            });
            let Some(event) = next else {
                break;
            };
            let mut index = 0;
            loop {
                let taken = {
                    let mut slots = self.slots.borrow_mut();
                    let Some(slot) = slots.get_mut(index) else {
                        break;
                    };
                    slot.listener.take().map(|listener| (slot.id, listener))
                };
                if let Some((id, mut listener)) = taken {
                    listener(&event);
                    let mut slots = self.slots.borrow_mut();
                    if let Some(slot) = slots.iter_mut().find(|slot| slot.id == id)
                        && !slot.removed
                    {
                        slot.listener = Some(listener);
                    }
                }
                index += 1;
            }
        }
        self.slots.borrow_mut().retain(|slot| !slot.removed);
        self.delivering.set(false);
    }
}

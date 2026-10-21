//! Delivery of a session's events.
//!
//! The events of one change of the session, one
//! [`SessionShared::update`](super::SessionShared), are a step: they reach
//! every listener together, once the change is complete, in the order the
//! changes happened, so a client applies a change whole or not at all
//! (`docs/decisions/0023-a-command-settles-on-the-state-it-changed.md`). A
//! listener may call back into the session; what that causes is delivered as
//! a step of its own once the current one has reached every listener.
//!
//! The end of a turn is one step: the answer's completed text with the phase
//! the closing save leaves. From the answer to that save the bus keeps the
//! steps that change the transcript or the phase, and releases them as one
//! step. Every other step passes, unless it carries an event of a kind the bus
//! keeps, whose later state would otherwise arrive before the earlier one. The
//! live reading of a request's output (`SessionEvent::OutputChanged`) belongs
//! to no client's state and always passes, as a step of its own.

use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    mem::discriminant,
};

use tokio::sync::Notify;

use super::SessionEvent;

type Listener = Box<dyn FnMut(&[SessionEvent])>;

struct Slot {
    id: u64,
    /// Taken out while it runs, so that it may call back into the session.
    listener: Option<Listener>,
    removed: bool,
}

#[derive(Default)]
pub(crate) struct EventBus {
    slots: RefCell<Vec<Slot>>,
    /// The steps waiting for delivery, in order.
    queue: RefCell<VecDeque<Vec<SessionEvent>>>,
    /// What a hold keeps until `release`, which delivers it as one step.
    kept: RefCell<Vec<SessionEvent>>,
    delivering: Cell<bool>,
    held: Cell<bool>,
    /// Told when the bus keeps nothing any more.
    released: Notify,
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

    /// Delivers the step `events` after those already waiting. A call made
    /// while a delivery runs only queues it: the running delivery delivers it
    /// in order. While the bus holds, a step that changes the transcript or the
    /// phase, or carries a kind the bus keeps, is kept.
    pub(crate) fn deliver(&self, events: Vec<SessionEvent>) {
        if events.is_empty() {
            return;
        }
        if self.held.get() {
            let (output, step): (Vec<_>, Vec<_>) =
                events.into_iter().partition(|event| matches!(event, SessionEvent::OutputChanged { .. }));
            if !output.is_empty() {
                self.queue.borrow_mut().push_back(output);
            }
            if !step.is_empty() {
                if self.keeps(&step) {
                    self.kept.borrow_mut().extend(step);
                } else {
                    self.queue.borrow_mut().push_back(step);
                }
            }
        } else {
            self.queue.borrow_mut().push_back(events);
        }
        self.drain();
    }

    /// Whether a hold keeps the step `step`.
    fn keeps(&self, step: &[SessionEvent]) -> bool {
        let kept = self.kept.borrow();
        step.iter().any(|event| {
            matches!(event, SessionEvent::TranscriptChanged { .. } | SessionEvent::PhaseChanged { .. })
                || kept.iter().any(|each| discriminant(each) == discriminant(event))
        })
    }

    /// Keeps the steps a hold keeps from now on until `release`. Called
    /// outside a delivery, so nothing given before it waits.
    pub(crate) fn hold(&self) {
        debug_assert!(!self.delivering.get(), "a hold starts outside a delivery");
        self.held.set(true);
    }

    /// Delivers what the bus kept, as one step, and every later step at once.
    pub(crate) fn release(&self) {
        if self.held.replace(false) {
            let kept = std::mem::take(&mut *self.kept.borrow_mut());
            if !kept.is_empty() {
                self.queue.borrow_mut().push_back(kept);
            }
            self.drain();
            self.released.notify_waiters();
        }
    }

    /// Resolves once the bus keeps nothing: every step given before the call
    /// has reached the listeners.
    pub(crate) async fn delivered(&self) {
        loop {
            let released = self.released.notified();
            if self.kept.borrow().is_empty() {
                return;
            }
            released.await;
        }
    }

    fn drain(&self) {
        if self.delivering.replace(true) {
            return;
        }
        loop {
            let Some(step) = self.queue.borrow_mut().pop_front() else {
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
                    listener(&step);
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

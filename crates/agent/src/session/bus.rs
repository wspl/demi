//! Delivery of a session's events (`runtime.md` § A turn): each event, once
//! its change is complete, reaches every listener in the order the changes
//! happened. A listener may call back into the session; what that causes is
//! delivered after the current event has reached every listener.

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
    delivering: Cell<bool>,
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
    /// them in order.
    pub(crate) fn deliver(&self, events: Vec<SessionEvent>) {
        if events.is_empty() {
            return;
        }
        self.queue.borrow_mut().extend(events);
        if self.delivering.replace(true) {
            return;
        }
        loop {
            let Some(event) = self.queue.borrow_mut().pop_front() else {
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

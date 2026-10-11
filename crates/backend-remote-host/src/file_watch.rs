//! A Host's subscriptions as the backend shares them (`web-api.md` § File
//! watch, `runner.md` § Watching files): one runner subscription per
//! subject on a connection, held by every page's watch that holds the same
//! subject. The first holder subscribes with `fs_subscribe`, the last one to
//! let go ends it with `fs_unsubscribe`, a holder's renewal renews it on the
//! runner, and the connection's end ends every holder.

use std::{
    cell::RefCell,
    collections::{BTreeSet, HashMap},
};

use demi_runner_protocol::files::Subject;
use demi_runner_protocol::wire::Inbound;
use tokio::sync::mpsc;

use crate::link::Link;

/// What a holder of a Host's subscriptions hears next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostEvent {
    /// The Host watches `subject` from `since`, a number of `epoch`.
    Live {
        subject: Subject,
        since: u64,
        epoch: String,
    },
    /// Subjects the holder holds that changed, each with the number of its
    /// latest change, from one batch of the runner's.
    Changed(Vec<(Subject, u64)>),
    /// The Host cannot watch `subject`, for this reason; it is over.
    Failed { subject: Subject, reason: String },
    /// The connection to the Host ended; nothing more comes.
    Ended,
}

/// One runner subscription and the holders that share it.
struct Shared {
    id: String,
    holders: BTreeSet<u64>,
    /// Once the runner said `fs_live`.
    live: Option<(u64, String)>,
}

/// A connection's runner subscriptions, by subject and by the id the
/// runner's messages name, and the holders that hear of them.
#[derive(Default)]
pub(crate) struct LinkSubscriptions {
    by_subject: HashMap<Subject, Shared>,
    by_id: HashMap<String, Subject>,
    holders: HashMap<u64, mpsc::UnboundedSender<HostEvent>>,
    next: u64,
}

impl LinkSubscriptions {
    /// The runner watches subscription `id`. A report of one that ended is
    /// one that crossed its `fs_unsubscribe`.
    pub(crate) fn live(&mut self, id: &str, since: u64, epoch: String) {
        let Some(subject) = self.by_id.get(id) else {
            return;
        };
        let shared = self.by_subject.get_mut(subject).expect("an id names a subject held");
        shared.live = Some((since, epoch.clone()));
        for holder in &shared.holders {
            if let Some(sender) = self.holders.get(holder) {
                // A holder that went has nothing to hear.
                let _ = sender.send(HostEvent::Live {
                    subject: subject.clone(),
                    since,
                    epoch: epoch.clone(),
                });
            }
        }
    }

    /// One batch of the runner's: each holder hears the subjects it holds
    /// among them, in one event.
    pub(crate) fn changed(&mut self, changes: Vec<(String, u64)>) {
        let mut by_holder: HashMap<u64, Vec<(Subject, u64)>> = HashMap::new();
        for (id, at) in changes {
            let Some(subject) = self.by_id.get(&id) else {
                continue;
            };
            let shared = &self.by_subject[subject];
            for holder in &shared.holders {
                by_holder.entry(*holder).or_default().push((subject.clone(), at));
            }
        }
        for (holder, changes) in by_holder {
            if let Some(sender) = self.holders.get(&holder) {
                let _ = sender.send(HostEvent::Changed(changes));
            }
        }
    }

    /// The runner cannot watch subscription `id`: its holders hear it, and a
    /// holder that subscribes again starts another.
    pub(crate) fn failed(&mut self, id: &str, reason: String) {
        let Some(subject) = self.by_id.remove(id) else {
            return;
        };
        let shared = self.by_subject.remove(&subject).expect("an id names a subject held");
        for holder in &shared.holders {
            if let Some(sender) = self.holders.get(holder) {
                let _ = sender.send(HostEvent::Failed {
                    subject: subject.clone(),
                    reason: reason.clone(),
                });
            }
        }
    }

    /// The connection ended: every holder hears it.
    pub(crate) fn end(&mut self) {
        for (_, sender) in self.holders.drain() {
            let _ = sender.send(HostEvent::Ended);
        }
        self.by_subject.clear();
        self.by_id.clear();
    }

    /// Lets `holder`'s hold of `subject` go: the message that ends the
    /// runner's subscription once nobody holds it.
    fn release(&mut self, holder: u64, subject: &Subject) -> Option<Inbound> {
        let shared = self.by_subject.get_mut(subject)?;
        shared.holders.remove(&holder);
        if !shared.holders.is_empty() {
            return None;
        }
        let shared = self.by_subject.remove(subject).expect("the subject was just held");
        self.by_id.remove(&shared.id);
        Some(Inbound::FsUnsubscribe { id: shared.id })
    }
}

/// One page's hold on a connection's subscriptions. Dropping it lets go of
/// all it holds.
pub struct HostSubscriber {
    link: Link,
    id: u64,
    held: RefCell<BTreeSet<Subject>>,
    events: mpsc::UnboundedReceiver<HostEvent>,
}

impl Link {
    /// A new holder of the connection's subscriptions. On a connection that
    /// ended, it hears `Ended` at once.
    pub(crate) fn subscriber(&self) -> HostSubscriber {
        let (sender, events) = mpsc::unbounded_channel();
        let id = self.with_subscriptions(|subscriptions| {
            subscriptions.next += 1;
            let id = subscriptions.next;
            if self.is_closed() {
                // Nobody is left to hear it, but the holder.
                let _ = sender.send(HostEvent::Ended);
            } else {
                subscriptions.holders.insert(id, sender);
            }
            id
        });
        HostSubscriber {
            link: self.clone(),
            id,
            held: RefCell::new(BTreeSet::new()),
            events,
        }
    }
}

impl HostSubscriber {
    /// Holds `subject`, subscribing on the runner unless another holder
    /// does: one that is live already is heard as live at once.
    pub fn hold(&self, subject: Subject) {
        if !self.held.borrow_mut().insert(subject.clone()) {
            return;
        }
        let holder = self.id;
        let posted = self.link.with_subscriptions(|subscriptions| {
            if let Some(shared) = subscriptions.by_subject.get_mut(&subject) {
                shared.holders.insert(holder);
                if let (Some((since, epoch)), Some(sender)) = (&shared.live, subscriptions.holders.get(&holder)) {
                    let _ = sender.send(HostEvent::Live {
                        subject: subject.clone(),
                        since: *since,
                        epoch: epoch.clone(),
                    });
                }
                return None;
            }
            let id = uuid::Uuid::new_v4().simple().to_string();
            subscriptions.by_id.insert(id.clone(), subject.clone());
            subscriptions.by_subject.insert(
                subject.clone(),
                Shared {
                    id: id.clone(),
                    holders: BTreeSet::from([holder]),
                    live: None,
                },
            );
            Some(Inbound::FsSubscribe {
                id,
                kind: subject.kind,
                path: subject.path.clone(),
            })
        });
        if let Some(message) = posted {
            self.link.post(&message);
        }
    }

    /// Lets go of `subject`.
    pub fn release(&self, subject: &Subject) {
        if !self.held.borrow_mut().remove(subject) {
            return;
        }
        let holder = self.id;
        let ended = self
            .link
            .with_subscriptions(|subscriptions| subscriptions.release(holder, subject));
        if let Some(message) = ended {
            self.link.post(&message);
        }
    }

    /// Renews the runner's leases of `subjects`, those this holder holds.
    pub fn renew(&self, subjects: &[Subject]) {
        let held = self.held.borrow();
        let ids: Vec<String> = self.link.with_subscriptions(|subscriptions| {
            subjects
                .iter()
                .filter(|subject| held.contains(*subject))
                .filter_map(|subject| subscriptions.by_subject.get(subject).map(|shared| shared.id.clone()))
                .collect()
        });
        if !ids.is_empty() {
            self.link.post(&Inbound::FsRenew { ids });
        }
    }

    /// What the Host says next of what this holder holds. After `Ended` it
    /// says the same again.
    pub async fn next(&mut self) -> HostEvent {
        loop {
            match self.events.recv().await {
                Some(HostEvent::Failed { subject, reason }) => {
                    if self.held.borrow_mut().remove(&subject) {
                        return HostEvent::Failed { subject, reason };
                    }
                }
                Some(event) => return event,
                None => return HostEvent::Ended,
            }
        }
    }
}

impl Drop for HostSubscriber {
    fn drop(&mut self) {
        let held = std::mem::take(&mut *self.held.borrow_mut());
        let holder = self.id;
        let ended: Vec<Inbound> = self.link.with_subscriptions(|subscriptions| {
            subscriptions.holders.remove(&holder);
            held.iter()
                .filter_map(|subject| subscriptions.release(holder, subject))
                .collect()
        });
        for message in ended {
            self.link.post(&message);
        }
    }
}

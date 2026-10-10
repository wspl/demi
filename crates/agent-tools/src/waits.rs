//! The `shell` calls that wait for their Host (`sessions-and-targets.md`
//! § Host operations, § Switch the primary target): a call whose primary
//! Host's runner is not connected waits for it without limit, its row reads
//! *Waiting for Old Laptop* meanwhile, and a move of the conversation ends it
//! as not run. A tree keeps one [`HostWaits`] for all its nodes: every call
//! in flight registers in it, so a move can tell a tree that works only by
//! waiting from one that works otherwise.

use std::cell::RefCell;
use std::rc::Rc;

use demi_conversation_socket_protocol::WaitingCall;
use demi_shared_types::NodeId;
use tokio::sync::watch;

/// Where a Host's resolution says that it waits for the Host's runner.
pub trait HostWait {
    /// The resolution waits for the runner of the Host named `host`, or,
    /// with none, no longer waits.
    fn waiting(&self, host: Option<&str>);
}

/// A resolution no call watches, such as a page's input to a running
/// command, which shows nothing while it waits.
impl HostWait for () {
    fn waiting(&self, _host: Option<&str>) {}
}

/// What a move does with a call that waits.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Hold {
    /// No move holds it.
    Free,
    /// A move runs and will end it, unless the move fails.
    Held,
    /// The move was made: the call ends with this result.
    Ended(Rc<str>),
}

/// A call in flight.
struct Entry {
    node: NodeId,
    call: String,
    /// The Host whose runner it waits for, while it does.
    host: RefCell<Option<String>>,
    hold: watch::Sender<Hold>,
}

/// A tree's calls in flight, and which of them wait for their Host.
pub struct HostWaits {
    calls: RefCell<Vec<Rc<Entry>>>,
    /// Told of each change of a node's waiting calls, which it reads back
    /// with [`waiting`](Self::waiting).
    changed: Box<dyn Fn(&HostWaits, &NodeId)>,
}

impl HostWaits {
    /// The calls of a tree, which tells `changed` each time a node's
    /// waiting calls change.
    pub fn new(changed: impl Fn(&HostWaits, &NodeId) + 'static) -> Rc<Self> {
        Rc::new(Self {
            calls: RefCell::default(),
            changed: Box::new(changed),
        })
    }

    /// Registers the call `call` of `node` while it runs; it goes with the
    /// answer.
    pub fn enter(self: &Rc<Self>, node: &NodeId, call: &str) -> CallWait {
        let entry = Rc::new(Entry {
            node: node.clone(),
            call: call.to_owned(),
            host: RefCell::new(None),
            hold: watch::Sender::new(Hold::Free),
        });
        self.calls.borrow_mut().push(entry.clone());
        CallWait {
            waits: self.clone(),
            entry,
        }
    }

    /// The calls of `node` that wait, each with the Host it waits for, in
    /// the order they started.
    pub fn waiting(&self, node: &NodeId) -> Vec<WaitingCall> {
        self.calls
            .borrow()
            .iter()
            .filter(|entry| entry.node == *node)
            .filter_map(|entry| {
                entry.host.borrow().as_ref().map(|host| WaitingCall {
                    tool_use_id: entry.call.clone(),
                    host: host.clone(),
                })
            })
            .collect()
    }

    /// Whether `node` has calls in flight and every one of them waits for
    /// its Host.
    pub fn only_waits(&self, node: &NodeId) -> bool {
        let calls = self.calls.borrow();
        let mut of_node = calls.iter().filter(|entry| entry.node == *node).peekable();
        of_node.peek().is_some() && of_node.all(|entry| entry.host.borrow().is_some())
    }

    /// Holds every call that waits for a move (§ Switch the primary target):
    /// a held call does not go on, even when its Host comes back, until the
    /// hold ends it or lets it go. None when no call waits.
    pub fn hold(&self) -> Option<HeldWaits> {
        let held: Vec<(Rc<Entry>, String)> = self
            .calls
            .borrow()
            .iter()
            .filter_map(|entry| {
                let host = entry.host.borrow().clone()?;
                Some((entry.clone(), host))
            })
            .collect();
        if held.is_empty() {
            return None;
        }
        for (entry, _) in &held {
            entry.hold.send_replace(Hold::Held);
        }
        Some(HeldWaits { held })
    }

    fn changed(&self, node: &NodeId) {
        (self.changed)(self, node);
    }
}

/// The calls a move holds. Dropped without [`end`](Self::end), as when the
/// move fails, it lets them go on waiting.
pub struct HeldWaits {
    /// Each call, with the Host it waited for when the move held it.
    held: Vec<(Rc<Entry>, String)>,
}

impl HeldWaits {
    /// Ends each held call with the result `result` makes of the Host it
    /// waited for. A call that ended meanwhile, as by a Stop, is gone and
    /// hears nothing.
    pub fn end(mut self, result: impl Fn(&str) -> String) {
        for (entry, host) in std::mem::take(&mut self.held) {
            entry.hold.send_replace(Hold::Ended(result(&host).into()));
        }
    }
}

impl Drop for HeldWaits {
    fn drop(&mut self) {
        for (entry, _) in &self.held {
            entry.hold.send_if_modified(|hold| {
                let held = *hold == Hold::Held;
                if held {
                    *hold = Hold::Free;
                }
                held
            });
        }
    }
}

/// A call's registration while it runs.
pub struct CallWait {
    waits: Rc<HostWaits>,
    entry: Rc<Entry>,
}

impl CallWait {
    /// Resolves once a move ended the call, with the call's result; never
    /// otherwise.
    pub async fn ended(&self) -> Rc<str> {
        let mut hold = self.entry.hold.subscribe();
        // The entry keeps the sender while this registration lives.
        let ended = hold
            .wait_for(|hold| matches!(hold, Hold::Ended(_)))
            .await
            .map(|hold| hold.clone());
        match ended {
            Ok(Hold::Ended(result)) => result,
            _ => std::future::pending().await,
        }
    }

    /// Once the call's Host is resolved: the result a move that holds the
    /// call ends it with, after waiting for that move; none when no move
    /// holds it or the move failed, and the call goes on.
    pub async fn settled(&self) -> Option<Rc<str>> {
        let mut hold = self.entry.hold.subscribe();
        // The entry keeps the sender while this registration lives.
        let settled = hold
            .wait_for(|hold| *hold != Hold::Held)
            .await
            .map(|hold| hold.clone());
        match settled {
            Ok(Hold::Ended(result)) => Some(result),
            _ => None,
        }
    }
}

impl HostWait for CallWait {
    fn waiting(&self, host: Option<&str>) {
        let changed = {
            let mut current = self.entry.host.borrow_mut();
            let changed = current.as_deref() != host;
            *current = host.map(str::to_owned);
            changed
        };
        if changed {
            self.waits.changed(&self.entry.node);
        }
    }
}

impl Drop for CallWait {
    fn drop(&mut self) {
        self.waits
            .calls
            .borrow_mut()
            .retain(|entry| !Rc::ptr_eq(entry, &self.entry));
        if self.entry.host.borrow().is_some() {
            self.waits.changed(&self.entry.node);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    fn node(id: &str) -> NodeId {
        NodeId::try_from(id.to_owned()).unwrap()
    }

    #[tokio::test]
    async fn a_held_call_whose_host_came_back_waits_for_the_move_and_a_failed_move_lets_it_go() {
        let told = Rc::new(Cell::new(0));
        let waits = HostWaits::new({
            let told = told.clone();
            move |_, _| told.set(told.get() + 1)
        });
        let root = node("root");
        let first = waits.enter(&root, "t1");
        let second = waits.enter(&root, "t2");
        assert!(!waits.only_waits(&root), "a call that does not wait is other work");
        first.waiting(Some("Old Laptop"));
        second.waiting(Some("Old Laptop"));
        assert!(waits.only_waits(&root));
        assert_eq!(told.get(), 2);

        // A move holds both; the first's Host comes back meanwhile, and the
        // move fails: the first goes on, the second waits again.
        let held = waits.hold().unwrap();
        first.waiting(None);
        {
            let settling = first.settled();
            tokio::pin!(settling);
            assert!(futures_util::poll!(&mut settling).is_pending(), "a held call waits for the move");
            drop(held);
            assert_eq!(settling.await, None);
        }

        // The next move is made and ends the call that still waits.
        let held = waits.hold().unwrap();
        held.end(|host| format!("moved from {host}"));
        assert_eq!(&*second.ended().await, "moved from Old Laptop");
        drop((first, second));
        assert!(waits.waiting(&root).is_empty());
        assert!(waits.hold().is_none());
    }
}

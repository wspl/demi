//! A tree's live output (`runtime.md` § Live output): the commands whose
//! pages' view changed and was not sent yet, the task that sends them to
//! every attached connection at most every [`INTERVAL`] and an end at once,
//! and the feed through which each node's shell environments report their
//! commands and learn whether a page watches.

use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
    time::Duration,
};

use demi_agent_tools::shell_output;
use demi_host_interface::{CommandRecord, PageFeed};
use demi_shared_types::{CommandId, NodeId};
use tokio::{
    sync::{Notify, watch},
    time::Instant,
};

use super::FrameSink;

/// How often a tree sends its commands' new output, at most.
pub(crate) const INTERVAL: Duration = Duration::from_millis(250);

/// The tree's commands whose new output waits for the next send.
pub(crate) struct LiveOutput {
    sink: Rc<FrameSink>,
    /// In the order their views changed; emptied when they are sent, when a
    /// command ends, and when the last connection detaches.
    waiting: RefCell<Vec<Waiting>>,
    /// When the last new output went.
    sent: Cell<Option<Instant>>,
    /// Wakes the sending task when a command starts to wait.
    wake: Notify,
}

/// A command whose new output waits: its view is read when it is sent.
struct Waiting {
    command: CommandId,
    /// The subagent that runs it; none for the root.
    subagent: Option<NodeId>,
    record: Rc<RefCell<CommandRecord>>,
}

impl LiveOutput {
    pub(crate) fn new(sink: Rc<FrameSink>) -> Rc<Self> {
        Rc::new(Self {
            sink,
            waiting: RefCell::default(),
            sent: Cell::new(None),
            wake: Notify::new(),
        })
    }

    /// The feed of a node: the subagent `subagent`, or the root when none.
    /// It does not keep the tree alive.
    pub(crate) fn feed(self: &Rc<Self>, subagent: Option<NodeId>) -> Rc<dyn PageFeed> {
        Rc::new(NodeFeed {
            live: Rc::downgrade(self),
            subagent,
        })
    }

    /// A command's view changed: new output waits for the next send, and an
    /// end goes at once, with nothing of the command after it. Without an
    /// attached connection nothing is sent or kept.
    fn changed(&self, subagent: &Option<NodeId>, record: &Rc<RefCell<CommandRecord>>) {
        if !self.sink.attached() {
            return;
        }
        let (command, running) = {
            let record = record.borrow();
            (record.command_id().clone(), record.is_running())
        };
        let mut waiting = self.waiting.borrow_mut();
        let known = waiting.iter().position(|entry| entry.command == command);
        if running {
            if known.is_none() {
                waiting.push(Waiting {
                    command,
                    subagent: subagent.clone(),
                    record: record.clone(),
                });
                self.wake.notify_one();
            }
            return;
        }
        if let Some(index) = known {
            waiting.remove(index);
        }
        drop(waiting);
        let view = record.borrow().page_view();
        self.sink.emit(shell_output(subagent.clone(), view));
    }

    /// When the waiting commands are due: at once after a quiet interval.
    fn due(&self) -> Option<Instant> {
        if self.waiting.borrow().is_empty() {
            return None;
        }
        Some(
            self.sent
                .get()
                .map_or_else(Instant::now, |sent| sent + INTERVAL),
        )
    }

    /// Sends each waiting command's view as it is now.
    fn send(&self) {
        let waiting = std::mem::take(&mut *self.waiting.borrow_mut());
        self.sent.set(Some(Instant::now()));
        for Waiting {
            subagent, record, ..
        } in waiting
        {
            let view = record.borrow().page_view();
            self.sink.emit(shell_output(subagent, view));
        }
    }
}

/// The tree's sending task: it sends the waiting commands at most every
/// [`INTERVAL`], keeps a timer only while commands wait, and forgets them
/// when the last connection detaches. It ends with the tree, which aborts
/// it.
pub(crate) async fn send_changes(live: Rc<LiveOutput>) {
    let mut attached = live.sink.watch_attached();
    loop {
        let due = live.due();
        tokio::select! {
            () = live.wake.notified() => {}
            () = tokio::time::sleep_until(due.unwrap_or_else(Instant::now)), if due.is_some() => live.send(),
            changed = attached.changed() => {
                // The sink lives as long as the tree that aborts this task.
                if changed.is_err() {
                    return;
                }
                if !*attached.borrow_and_update() {
                    live.waiting.borrow_mut().clear();
                }
            }
        }
    }
}

/// A node's feed of its tree's live output.
struct NodeFeed {
    live: Weak<LiveOutput>,
    subagent: Option<NodeId>,
}

impl PageFeed for NodeFeed {
    fn changed(&self, record: &Rc<RefCell<CommandRecord>>) {
        if let Some(live) = self.live.upgrade() {
            live.changed(&self.subagent, record);
        }
    }

    fn watching(&self) -> watch::Receiver<bool> {
        match self.live.upgrade() {
            Some(live) => live.sink.watch_attached(),
            // A tree that is gone has no page; its sender is gone too.
            None => watch::channel(false).1,
        }
    }
}

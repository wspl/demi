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

use demi_core::{CommandId, NodeId};
use demi_shell::{CommandRecord, PageFeed};
use tokio::{
    sync::{Notify, watch},
    time::Instant,
};

use super::FrameSink;
use crate::tools;

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
        self.sink.emit(tools::shell_output(subagent.clone(), view));
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
            self.sink.emit(tools::shell_output(subagent, view));
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

#[cfg(test)]
mod tests {
    use demi_agent_protocol::{ServerFrame, ShellStatus};
    use demi_core::{ShellId, StreamKind};
    use demi_shell::Ending;
    use tokio_util::task::AbortOnDropHandle;

    use super::*;
    use crate::server::{
        connection::{FrameRx, Outbox, Outgoing},
        tree::Attachment,
    };

    fn record(command: &str) -> Rc<RefCell<CommandRecord>> {
        Rc::new(RefCell::new(CommandRecord::new(
            ShellId::try_from("shell").unwrap(),
            CommandId::try_from(command).unwrap(),
            format!("call-{command}"),
        )))
    }

    fn print(record: &Rc<RefCell<CommandRecord>>, text: &str) {
        record.borrow_mut().append_output(StreamKind::Stdout, text);
    }

    /// Lets the sending task run.
    async fn settle() {
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
    }

    /// The command views waiting in `frames`: the subagent, the command,
    /// whether it runs, and its tail.
    fn views(frames: &mut FrameRx) -> Vec<(Option<String>, String, bool, String)> {
        let mut views = Vec::new();
        while let Some(outgoing) = frames.try_recv() {
            let Outgoing::Frame(ServerFrame::ShellOutput {
                subagent_id,
                status,
            }) = outgoing
            else {
                panic!("expected a command's view, got {outgoing:?}");
            };
            let command = status.command();
            views.push((
                subagent_id.map(|id| id.to_string()),
                command.command_id.to_string(),
                matches!(*status, ShellStatus::Running { .. }),
                command.tail.clone(),
            ));
        }
        views
    }

    fn view(
        subagent: Option<&str>,
        command: &str,
        running: bool,
        tail: &str,
    ) -> (Option<String>, String, bool, String) {
        (
            subagent.map(str::to_owned),
            command.to_owned(),
            running,
            tail.to_owned(),
        )
    }

    /// The sending rules of `runtime.md` § Live output, on a paused clock: a
    /// quiet change goes at once, new output at most every interval with
    /// the tree's changed commands together, an end at once with nothing of
    /// its command after it; a chatty command sends a frame an interval;
    /// without a page nothing is sent or kept, and the jobs are followed
    /// only while a page is attached.
    #[tokio::test(flavor = "local", start_paused = true)]
    async fn new_output_goes_at_most_every_interval_and_an_end_at_once() {
        let sink = Rc::new(FrameSink::new());
        let live = LiveOutput::new(sink.clone());
        let _sending = AbortOnDropHandle::new(tokio::task::spawn_local(send_changes(live.clone())));
        let root = live.feed(None);
        let child = live.feed(Some(NodeId::try_from("child").unwrap()));
        let mut following = root.watching();
        assert!(!*following.borrow_and_update());
        let (outbox, mut frames) = Outbox::new(64);
        sink.attach(Attachment {
            connection: 1,
            outbox,
        });
        assert!(*following.borrow_and_update());

        // A change after a quiet interval goes at once.
        let a = record("a");
        root.changed(&a);
        settle().await;
        assert_eq!(views(&mut frames), [view(None, "a", true, "")]);

        // Within the interval, changes wait and go together, each command's
        // view as it is when they go.
        print(&a, "one\n");
        root.changed(&a);
        print(&a, "two\n");
        root.changed(&a);
        let b = record("b");
        child.changed(&b);
        settle().await;
        assert_eq!(views(&mut frames), []);
        tokio::time::advance(INTERVAL).await;
        settle().await;
        assert_eq!(
            views(&mut frames),
            [
                view(None, "a", true, "one\ntwo\n"),
                view(Some("child"), "b", true, "")
            ]
        );

        // An end goes at once, with what waited of its command; nothing of
        // the command follows it.
        print(&a, "three\n");
        root.changed(&a);
        let ended = a.borrow_mut().settle(
            Ending::Exited(0),
            String::new(),
            String::new(),
            None,
            "end\n",
        );
        assert!(ended);
        root.changed(&a);
        assert_eq!(
            views(&mut frames),
            [view(None, "a", false, "one\ntwo\nthree\nend\n")]
        );
        tokio::time::advance(INTERVAL).await;
        settle().await;
        assert_eq!(views(&mut frames), []);

        // A command that prints without a pause sends a frame an interval.
        for _ in 0..1000 {
            print(&b, "x");
            child.changed(&b);
            tokio::time::advance(std::time::Duration::from_millis(1)).await;
        }
        settle().await;
        let sent = views(&mut frames);
        assert!(
            (4..=5).contains(&sent.len()),
            "{} frames in a second",
            sent.len()
        );

        // Without a page nothing is sent or kept, and the jobs are no longer
        // followed: what waited when the last page left, and what changes
        // while none is attached, never reaches the next page.
        print(&b, "y");
        child.changed(&b);
        sink.detach(1);
        assert!(!*following.borrow_and_update());
        settle().await;
        print(&b, "z");
        child.changed(&b);
        let (outbox, mut later) = Outbox::new(64);
        sink.attach(Attachment {
            connection: 2,
            outbox,
        });
        tokio::time::advance(INTERVAL).await;
        settle().await;
        assert_eq!(views(&mut later), []);
    }
}

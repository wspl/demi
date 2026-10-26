//! Each node's request output, as the pages that follow the node see it beside
//! the chips row.
//!
//! A node's session counts the output of the request it runs and announces
//! each change. The tree sends the pages that follow the node a request's
//! start, its end and the turn's end at once, and the counting in between at
//! most every [`INTERVAL`]: the latest reading, its streaming time grown by how
//! long it waited, so the time is the time it leaves. A page reads the
//! reading of a node with the node's latest page.
//!
//! The timer runs only while a change waits. The sending task ends with the
//! tree, which aborts it, and holds nothing of a session, so a closed tree's
//! sessions go with the tree.

use std::{
    cell::RefCell,
    collections::HashMap,
    rc::Rc,
    time::Duration,
};

use demi_conversation_lane_protocol::{RequestOutput, ServerFrame};
use demi_shared_types::NodeId;
use tokio::{sync::Notify, time::Instant};

use super::FrameSink;

/// How often the reading of a streaming request goes, at most.
pub(crate) const INTERVAL: Duration = Duration::from_millis(500);

/// What of a node's reading the pages were sent.
struct Sent {
    /// The request of the last frame sent and whether it had ended.
    request: (String, bool),
    /// When the last frame went.
    at: Instant,
    /// The change of the streaming request that waits for the next send, and
    /// when the session announced it.
    waiting: Option<(RequestOutput, Instant)>,
}

/// The readings of the tree's nodes, and what of them the pages were sent. A
/// node that has no reading has no entry.
pub(crate) struct OutputReadings {
    sink: Rc<FrameSink>,
    nodes: RefCell<HashMap<NodeId, Sent>>,
    /// Wakes the sending task when a change starts to wait.
    wake: Notify,
}

impl OutputReadings {
    pub(crate) fn new(sink: Rc<FrameSink>) -> Rc<Self> {
        Rc::new(Self {
            sink,
            nodes: RefCell::new(HashMap::new()),
            wake: Notify::new(),
        })
    }

    /// The session of `node` announced `output`: another request, its end or
    /// the turn's goes at once, more of the same request with the next send.
    pub(crate) fn changed(&self, node: &NodeId, output: &Option<RequestOutput>) {
        let mut nodes = self.nodes.borrow_mut();
        if let (Some(output), Some(sent)) = (output, nodes.get_mut(node))
            && sent.request == key(output)
        {
            if sent.waiting.replace((output.clone(), Instant::now())).is_none() {
                self.wake.notify_one();
            }
            return;
        }
        match output {
            Some(output) => {
                nodes.insert(
                    node.clone(),
                    Sent {
                        request: key(output),
                        at: Instant::now(),
                        waiting: None,
                    },
                );
            }
            None => {
                nodes.remove(node);
            }
        }
        drop(nodes);
        self.send(node, output.clone());
    }

    /// When the earliest waiting change is due: at once after a quiet
    /// interval.
    fn due(&self) -> Option<Instant> {
        self.nodes
            .borrow()
            .values()
            .filter(|sent| sent.waiting.is_some())
            .map(|sent| sent.at + INTERVAL)
            .min()
    }

    /// Sends every waiting change that is due, its streaming time as it is
    /// now.
    fn send_due(&self) {
        let now = Instant::now();
        let mut due = Vec::new();
        {
            let mut nodes = self.nodes.borrow_mut();
            for (node, sent) in nodes.iter_mut() {
                if sent.at + INTERVAL > now {
                    continue;
                }
                let Some((mut output, announced)) = sent.waiting.take() else {
                    continue;
                };
                let waited = u64::try_from(announced.elapsed().as_millis()).unwrap_or(u64::MAX);
                output.streamed_ms = output.streamed_ms.map(|streamed| streamed.saturating_add(waited));
                sent.at = now;
                due.push((node.clone(), output));
            }
        }
        for (node, output) in due {
            self.send(&node, Some(output));
        }
    }

    fn send(&self, node: &NodeId, output: Option<RequestOutput>) {
        self.sink.emit_to(
            node,
            ServerFrame::RequestOutput {
                node: node.clone(),
                output,
            },
        );
    }
}

fn key(output: &RequestOutput) -> (String, bool) {
    (output.request_id.clone(), output.ended)
}

/// The tree's task that sends the waiting changes of the readings,
/// [`INTERVAL`] after the last frame of each.
pub(crate) async fn send_changes(readings: Rc<OutputReadings>) {
    loop {
        // A frame that went at once meanwhile moves the time a waiting change
        // is due, so it is read again after each sleep.
        match readings.due() {
            None => readings.wake.notified().await,
            Some(due) if due > Instant::now() => tokio::time::sleep_until(due).await,
            Some(_) => readings.send_due(),
        }
    }
}

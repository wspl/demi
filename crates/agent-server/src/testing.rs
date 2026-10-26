//! What the compaction fixture's harness (`fixtures/compaction`) builds on: a
//! client that drives one connection the way a socket would, and the parts of
//! a product the server needs and a harness leaves empty.

use std::{collections::HashMap, rc::Rc, time::Duration};

use demi_conversation_lane_protocol::{AgentEntry, CacheChart, ClientContent, ClientFrame, ServerFrame};
use demi_agent_tools::HostResolver;
use demi_shared_types::{NodeId, PictureFailure, SessionPhase};
use futures_util::future::LocalBoxFuture;

use crate::{
    AgentServer, AgentsSource, Connection, ContentError, ContentResolver, FileReference, FrameRx, KeptAgents, Outgoing,
    ReplyFiles, ResolvedFiles,
};

/// A message's content of one text, as the web app sends it.
pub fn client_text(text: &str) -> Vec<ClientContent> {
    vec![ClientContent::Text {
        text: text.to_owned(),
    }]
}

/// A backend that holds no upload and pairs no device: every file reference
/// is refused.
struct NoFiles;

impl ContentResolver for NoFiles {
    fn resolve<'a>(
        &'a self,
        files: Vec<FileReference>,
    ) -> LocalBoxFuture<'a, Result<ResolvedFiles, ContentError>> {
        let refused = |message: String| ContentError {
            message,
            code: Some("frame_delivery_failed".to_owned()),
        };
        Box::pin(async move {
            match files.into_iter().next() {
                None => Ok(ResolvedFiles::default()),
                Some(FileReference::Upload { r#ref, .. }) => Err(refused(format!("upload {ref} is not available"))),
                Some(FileReference::RemoteFile { device_id, .. }) => {
                    Err(refused(format!("device {device_id} is not paired")))
                }
            }
        })
    }
}

/// A product that keeps nothing of a conversation's agents: no cost, no
/// side question's card.
pub struct NoKeptAgents;

impl AgentsSource for NoKeptAgents {
    fn kept<'a>(&'a self, _root: &'a NodeId) -> LocalBoxFuture<'a, KeptAgents> {
        Box::pin(async { KeptAgents::default() })
    }

    fn charts<'a>(&'a self, _root: &'a NodeId) -> LocalBoxFuture<'a, HashMap<u64, CacheChart>> {
        Box::pin(async { HashMap::new() })
    }
}

/// A Host without files: no path a reply writes is a file, and no picture is
/// copied.
pub struct NoHostFiles;

impl ReplyFiles for NoHostFiles {
    fn files<'a>(
        &'a self,
        _root: &'a NodeId,
        _cwd: &'a str,
        _paths: Vec<String>,
    ) -> LocalBoxFuture<'a, Option<HashMap<String, String>>> {
        Box::pin(async { Some(HashMap::new()) })
    }

    fn copy_picture<'a>(
        &'a self,
        _root: &'a NodeId,
        _node: &'a NodeId,
        _device: Option<&'a str>,
        _cwd: &'a str,
        _path: &'a str,
    ) -> LocalBoxFuture<'a, Result<String, PictureFailure>> {
        Box::pin(async { Err(PictureFailure::Missing) })
    }
}

/// The entry of the agent `id` an `agent` frame carries.
fn entry_in<'a>(frame: &'a ServerFrame, id: &NodeId) -> Option<&'a AgentEntry> {
    match frame {
        ServerFrame::Agent { agent, .. } if &agent.id == id => Some(agent),
        _ => None,
    }
}

/// How long a wait for a frame may take before it fails as a hang: the
/// scripted model answers at once.
const HANG_GUARD: Duration = Duration::from_secs(10);

/// A client of one connection: it hands frames to the connection as the
/// backend's socket task would, and reads what the outbox sends back.
pub struct TestClient<H: HostResolver> {
    root: NodeId,
    /// The root's phase as the frames read so far tell it.
    root_phase: std::cell::Cell<SessionPhase>,
    server: Rc<AgentServer<H>>,
    connection: Connection<H>,
    frames: FrameRx,
    /// The frames of a step read but not yet answered by `next`.
    read: std::collections::VecDeque<ServerFrame>,
    /// How long a wait for a frame may take before it fails as a hang.
    hang_guard: Duration,
}

impl<H: HostResolver> TestClient<H> {
    /// A client of the conversation `root`, whose frames refer to no file.
    pub fn connect(server: &Rc<AgentServer<H>>, root: &NodeId) -> Self {
        let (connection, frames) = server.connect(root.clone(), Rc::new(NoFiles));
        Self {
            root: root.clone(),
            root_phase: std::cell::Cell::new(SessionPhase::Idle),
            server: server.clone(),
            connection,
            frames,
            read: std::collections::VecDeque::new(),
            hang_guard: HANG_GUARD,
        }
    }

    /// This client with waits that fail as a hang only after `guard`, for a
    /// caller that waits on something slower than a scripted model, such as
    /// a real one.
    pub fn with_hang_guard(mut self, guard: Duration) -> Self {
        self.hang_guard = guard;
        self
    }

    /// Opens the conversation and follows its agents and its root, as a page
    /// that shows the conversation does having read them: `opened` carries no
    /// data.
    ///
    /// The client takes the root's phase from the agents a page reads next.
    pub async fn open(&self) {
        self.send(ClientFrame::Open { draft: None, follows: Vec::new() }).await;
        self.connection.follows().follow_agents();
        self.connection.follows().follow_node(self.root.clone());
        if let Some(entry) = self.read_agents().iter().find_map(|frame| entry_in(frame, &self.root)) {
            self.root_phase.set(entry.phase);
        }
    }

    /// What a page reads of the agents now: one `agent` frame of each entry,
    /// at the collection's version, as the changes that follow would carry them.
    /// None while the tree is not live.
    fn read_agents(&self) -> Vec<ServerFrame> {
        let Some(tree) = self.server.tree(&self.root) else {
            return Vec::new();
        };
        let (version, agents) = tree.agents();
        agents
            .into_iter()
            .map(|agent| ServerFrame::Agent { version: version.clone(), agent })
            .collect()
    }

    /// Hands `frame` to the connection and waits until it is handled.
    pub async fn send(&self, frame: ClientFrame) {
        self.connection.handle(frame).await;
    }

    /// The next frame; `None` once the outbox is closed or lagged.
    async fn next(&mut self) -> Option<ServerFrame> {
        if self.read.is_empty() {
            match self.frames.recv().await {
                Outgoing::Frames(step) => self.read.extend(step),
                Outgoing::Lagged | Outgoing::Closed => return None,
            }
        }
        let frame = self.read.pop_front()?;
        if let Some(entry) = entry_in(&frame, &self.root) {
            self.root_phase.set(entry.phase);
        }
        Some(frame)
    }

    /// The frames up to the one that tells the root went idle, after the
    /// frames read so far or these told it working: the end of its turn or
    /// its action, as a page sees it.
    pub async fn until_idle(&mut self) -> Vec<ServerFrame> {
        let worked = std::cell::Cell::new(self.root_phase.get() != SessionPhase::Idle);
        let root = self.root.clone();
        self.next_until(move |frame| match entry_in(frame, &root).map(|entry| entry.phase) {
            Some(SessionPhase::Idle) => worked.get(),
            Some(_) => {
                worked.set(true);
                false
            }
            None => false,
        })
        .await
    }

    /// The frames up to and including the first that `until` accepts. The
    /// wait fails, listing the frames it saw, when the connection closes
    /// first or when the client's hang guard runs out.
    pub async fn next_until(&mut self, until: impl Fn(&ServerFrame) -> bool) -> Vec<ServerFrame> {
        let guard = self.hang_guard;
        let mut frames = Vec::new();
        let waited = tokio::time::timeout(guard, async {
            while let Some(frame) = self.next().await {
                let done = until(&frame);
                frames.push(frame);
                if done {
                    return true;
                }
            }
            false
        })
        .await;
        match waited {
            Ok(true) => frames,
            Ok(false) => panic!("the connection closed before a frame ended the wait: {frames:#?}"),
            Err(_) => panic!("no frame ended the wait within {guard:?}: {frames:#?}"),
        }
    }
}

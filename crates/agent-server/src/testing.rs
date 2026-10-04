//! Test support (feature `testing`): provider runtimes that play scripts,
//! uploads a test resolves, and a client that drives one connection the way a
//! socket would. No test calls a real model.

use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap},
    rc::Rc,
    time::Duration,
};

use demi_agent_store::{media::HeldMedia, testing::test_model};
use demi_agent_tools::{HostResolver, ProfileModel, Unavailable};
use demi_conversation_socket_protocol::{ClientContent, ClientFrame, ServerFrame};
use demi_provider_common::{ProviderRuntime, testing::ScriptedRuntime};
use demi_shared_types::{ModelSelection, NodeId, ProviderModel, UnavailableSetting, UserContentBlock};
use futures_util::future::LocalBoxFuture;

use crate::{
    AgentServer, Connection, ContentError, ContentResolver, FileReference, FrameRx, Outgoing,
    ProviderResolver, ResolveError, ResolvedFiles,
};

/// A message's content of one text, as the web app sends it.
pub fn client_text(text: &str) -> Vec<ClientContent> {
    vec![ClientContent::Text {
        text: text.to_owned(),
    }]
}

/// Makes one provider's runtimes.
type Runtimes = Rc<dyn Fn() -> Box<dyn ProviderRuntime>>;

/// Provider runtimes by provider id: every runtime of one provider plays
/// that provider's one script, and a provider without one is unknown. Every
/// conversation's record holds the selection a test chose last, else
/// [`test_model`]. A provider's catalog lists the models a test gave it.
#[derive(Default)]
pub struct ScriptedProviders {
    runtimes: RefCell<HashMap<String, Runtimes>>,
    catalogs: RefCell<HashMap<String, Vec<ProviderModel>>>,
    selection: RefCell<Option<ModelSelection>>,
    /// Every resolution asked for: the conversation and the provider.
    pub calls: RefCell<Vec<(NodeId, String)>>,
}

impl ScriptedProviders {
    /// The selection every conversation's tree opens with from now on, as
    /// its record would hold it.
    pub fn select(&self, model: ModelSelection) {
        self.selection.replace(Some(model));
    }

    pub fn provide(&self, provider: &str, script: &ScriptedRuntime) {
        self.provide_runtime(provider, script.clone());
    }

    /// Every runtime of `provider` is a copy of `runtime`, such as one that
    /// answers each node of a tree from a script of its own.
    pub fn provide_runtime(&self, provider: &str, runtime: impl ProviderRuntime + Clone + 'static) {
        self.runtimes.borrow_mut().insert(
            provider.to_owned(),
            Rc::new(move || Box::new(runtime.clone()) as Box<dyn ProviderRuntime>),
        );
    }

    /// `provider`'s catalog lists `models` from now on, in place of what it
    /// listed.
    pub fn list(&self, provider: &str, models: Vec<ProviderModel>) {
        self.catalogs
            .borrow_mut()
            .insert(provider.to_owned(), models);
    }

    /// `provider` is gone from now on: it has no runtime and no catalog.
    pub fn remove(&self, provider: &str) {
        self.runtimes.borrow_mut().remove(provider);
        self.catalogs.borrow_mut().remove(provider);
    }
}

impl ProviderResolver for ScriptedProviders {
    fn selection<'a>(
        &'a self,
        _root: &'a NodeId,
    ) -> LocalBoxFuture<'a, Result<ModelSelection, ResolveError>> {
        let selection = self.selection.borrow().clone().unwrap_or_else(test_model);
        Box::pin(async move { Ok(selection) })
    }

    fn runtime<'a>(
        &'a self,
        root: &'a NodeId,
        model: &'a ModelSelection,
    ) -> LocalBoxFuture<'a, Result<Box<dyn ProviderRuntime>, ResolveError>> {
        self.calls
            .borrow_mut()
            .push((root.clone(), model.provider_id.clone()));
        let runtimes = self.runtimes.borrow().get(&model.provider_id).cloned();
        let provider = model.provider_id.clone();
        Box::pin(async move {
            let runtimes = runtimes.ok_or(ResolveError::Unknown(provider))?;
            Ok(runtimes())
        })
    }

    /// The model's own window: no user limits it here.
    fn context_window<'a>(&'a self, model: &'a ModelSelection) -> LocalBoxFuture<'a, u32> {
        let window = model.model.context_window;
        Box::pin(async move { window })
    }

    /// The selection the provider's catalog makes of `model`, as a backend
    /// makes it: an entry without a runtime is gone.
    fn profile_selection<'a>(
        &'a self,
        model: &'a ProfileModel,
    ) -> LocalBoxFuture<'a, Result<ModelSelection, Unavailable>> {
        let provider = &model.provider_id;
        let known = self.runtimes.borrow().contains_key(provider);
        let listed = self.catalogs.borrow().get(provider).and_then(|catalog| {
            catalog
                .iter()
                .find(|listed| listed.id == model.model_id)
                .cloned()
        });
        let selection = (|| {
            if !known {
                return Err(Unavailable::Entry);
            }
            let listed = listed.ok_or_else(|| Unavailable::Model(model.model_id.clone()))?;
            let missing = |setting| match setting {
                UnavailableSetting::Effort(effort) => Unavailable::Effort {
                    model: model.model_id.clone(),
                    effort,
                },
                UnavailableSetting::Tier(tier) => Unavailable::Tier {
                    model: model.model_id.clone(),
                    tier,
                },
            };
            let thinking = listed
                .thinking_for(model.thinking_effort.as_deref())
                .map_err(missing)?;
            let tier = listed
                .tier_for(model.service_tier_id.as_deref())
                .map_err(missing)?;
            Ok(listed.selection(provider, thinking, tier))
        })();
        Box::pin(async move { selection })
    }
}

/// Uploads a test gave the blocks they resolve to, with the bytes of their
/// media; every other file reference is refused, as a backend refuses one
/// it does not hold.
#[derive(Debug, Default)]
pub struct TestFiles {
    uploads: RefCell<BTreeMap<String, (Vec<UserContentBlock>, HeldMedia)>>,
}

impl TestFiles {
    pub fn new() -> Rc<Self> {
        Rc::new(Self::default())
    }

    /// The upload `reference` resolves to `blocks`, whose media reference
    /// the blobs of `media`'s bytes, as [`upload_blocks`](demi_agent_store::attachments::upload_blocks)
    /// answers them.
    pub fn upload(&self, reference: &str, (blocks, media): (Vec<UserContentBlock>, HeldMedia)) {
        self.uploads
            .borrow_mut()
            .insert(reference.to_owned(), (blocks, media));
    }
}

impl ContentResolver for TestFiles {
    fn resolve<'a>(
        &'a self,
        files: Vec<FileReference>,
    ) -> LocalBoxFuture<'a, Result<ResolvedFiles, ContentError>> {
        let refused = |message: String| ContentError {
            message,
            code: Some("frame_delivery_failed".to_owned()),
        };
        Box::pin(async move {
            let mut resolved = ResolvedFiles::default();
            for file in files {
                let (blocks, media) = match file {
                    FileReference::Upload { r#ref, .. } => {
                        let uploaded = self.uploads.borrow().get(&r#ref).cloned();
                        uploaded.ok_or_else(|| refused(format!("upload {ref} is not available")))?
                    }
                    FileReference::RemoteFile { device_id, .. } => {
                        return Err(refused(format!("device {device_id} is not paired")));
                    }
                };
                resolved.blocks.push(blocks);
                resolved.media.absorb(media);
            }
            Ok(resolved)
        })
    }
}

/// How long a test's wait for a frame may take before it fails as a hang
/// (`testing.md` § Time and stability): the scripted model answers at once.
pub const HANG_GUARD: Duration = Duration::from_secs(10);

/// A client of one connection: it hands frames to the connection as the
/// backend's socket task would, and reads what the outbox sends back.
pub struct TestClient<H: HostResolver> {
    connection: Connection<H>,
    frames: FrameRx,
    /// How long a wait for a frame may take before it fails as a hang.
    hang_guard: Duration,
}

impl<H: HostResolver> TestClient<H> {
    /// A client of the conversation `root`, whose new tree works in `cwd`,
    /// whose frames refer to no file.
    pub fn connect(server: &Rc<AgentServer<H>>, root: &NodeId, cwd: &str) -> Self {
        Self::connect_with(server, root, cwd, TestFiles::new())
    }

    /// A client whose frames' files `files` resolves.
    pub fn connect_with(
        server: &Rc<AgentServer<H>>,
        root: &NodeId,
        cwd: &str,
        files: Rc<dyn ContentResolver>,
    ) -> Self {
        let (connection, frames) = server.connect(root.clone(), cwd.to_owned(), files);
        Self {
            connection,
            frames,
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

    /// Hands `frame` to the connection and waits until it is handled.
    pub async fn send(&self, frame: ClientFrame) {
        self.connection.handle(frame).await;
    }

    /// The next frame; `None` once the outbox is closed or lagged.
    pub async fn next(&mut self) -> Option<ServerFrame> {
        match self.frames.recv().await {
            Outgoing::Frame(frame) => Some(frame),
            Outgoing::Lagged | Outgoing::Closed => None,
        }
    }

    /// Every frame waiting now.
    pub fn received(&mut self) -> Vec<ServerFrame> {
        waiting_frames(&mut self.frames)
    }

    /// The connection and its outbox apart, for a test that looks at what
    /// the page has heard while a frame of its is still being handled.
    pub fn split(&mut self) -> (&Connection<H>, &mut FrameRx) {
        (&self.connection, &mut self.frames)
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

    /// What the outbox has next, lagged or closed included.
    pub async fn outgoing(&mut self) -> Outgoing {
        self.frames.recv().await
    }

    pub fn connection(&self) -> &Connection<H> {
        &self.connection
    }
}

/// Every frame `outbox` holds now.
pub fn waiting_frames(outbox: &mut FrameRx) -> Vec<ServerFrame> {
    let mut frames = Vec::new();
    while let Some(Outgoing::Frame(frame)) = outbox.try_recv() {
        frames.push(frame);
    }
    frames
}

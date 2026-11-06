//! What the user's runners report of their projects' linked worktrees
//! (`crates/core-store/src/selection.rs` § Worktrees).
//!
//! Each connected runner is told the directories of its Host's projects when
//! it connects and whenever they change, and reports each one's worktrees
//! (`crates/runner-host/src/worktrees.rs`). A report reaches the core before
//! the runner's next message, which a move may be; the core stores the
//! reports one after another, and a move waits for the ones that came before
//! it ([`Core::worktrees_stored`]), so it selects by what the Host said, never
//! asking the Host.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};

use demi_core_store::Worktree;
use demi_core_sync::Part;
use demi_runner_protocol::wire::LinkedWorktree;
use demi_web_api_protocol::ids::DeviceId;
use futures_util::FutureExt as _;
use futures_util::future::{LocalBoxFuture, Shared};

use demi_core_host_access::root_of;
use crate::Core;

/// The core's part.
#[derive(Default)]
pub(crate) struct WorktreesHeld {
    /// The project directories each runner was told last.
    told: RefCell<HashMap<DeviceId, BTreeSet<String>>>,
    /// The last report's store, which the next one and a move wait for.
    stored: RefCell<Option<Shared<LocalBoxFuture<'static, ()>>>>,
}

impl Core {
    /// The user's projects changed: every connected runner whose projects'
    /// directories differ from what it was told hears them.
    pub(crate) async fn projects_changed(&self) {
        let projects = match self.store().projects().await {
            Ok(projects) => projects,
            Err(error) => {
                // The runners keep the directories they were told last,
                // until the next change or connection tells them again.
                tracing::warn!(event = "worktrees.projects_announce_failed", error = &error as &dyn std::error::Error, "the runners were not told of the user's projects");
                return;
            }
        };
        let mut directories: HashMap<DeviceId, BTreeSet<String>> = HashMap::new();
        for project in projects {
            directories.entry(project.device).or_default().insert(project.path);
        }
        let mut told = self.worktrees.told.borrow_mut();
        told.retain(|device, _| self.devices().connected(device));
        // A Host whose last project went is told it has none, so that one
        // made again in the same directory is new to it.
        for device in told.keys() {
            directories.entry(device.clone()).or_default();
        }
        for (device, directories) in directories {
            let Some(link) = self.devices().link(&device) else {
                continue;
            };
            if told.get(&device) == Some(&directories) {
                continue;
            }
            link.projects(directories.iter().cloned().collect());
            told.insert(device, directories);
        }
    }

    /// `device`'s runner connected: it holds no directories, so it is told
    /// them again, and reports every project's worktrees.
    pub(crate) async fn projects_presence(&self, device: &DeviceId) {
        self.worktrees.told.borrow_mut().remove(device);
        self.projects_changed().await;
    }

    /// `device`'s runner reported the worktrees of its project `directory`:
    /// stored after the reports before it, and the conversations a worktree's
    /// own project held go into the project.
    pub(crate) fn worktrees_reported(&self, device: &DeviceId, directory: String, worktrees: Vec<LinkedWorktree>) {
        let core = self.this();
        let device = device.clone();
        let before = self.worktrees.stored.borrow_mut().take();
        let store = async move {
            if let Some(before) = before {
                before.await;
            }
            let worktrees = worktrees
                .into_iter()
                .map(|worktree| Worktree { name: worktree.name, path: worktree.path })
                .collect();
            let stored = match core.store().store_worktrees(device.clone(), directory, worktrees).await {
                Ok(stored) => stored,
                Err(error) => {
                    tracing::warn!(event = "worktrees.store_failed", host_id = %device, error = &error as &dyn std::error::Error, "a project's worktrees were not stored");
                    return;
                }
            };
            if stored.projects {
                core.mark(Part::Projects);
            }
            for conversation in &stored.saved {
                core.mark(Part::Metadata(conversation.clone()));
            }
            for conversation in &stored.conversations {
                core.mark(Part::Conversation(conversation.clone()));
                // It works in the worktree now, so what it kept of its
                // project's files is let go, and its next request names it.
                core.project_files().release(conversation);
                core.agent().placement_changed(&root_of(conversation)).await;
            }
        }
        .boxed_local()
        .shared();
        *self.worktrees.stored.borrow_mut() = Some(store.clone());
        self.tasks().spawn_local(store);
    }

    /// Waits until the reports of worktrees that came before have been
    /// stored, so a move selects by them.
    pub(crate) async fn worktrees_stored(&self) {
        let stored = self.worktrees.stored.borrow().clone();
        if let Some(stored) = stored {
            stored.await;
        }
    }
}

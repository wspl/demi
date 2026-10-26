//! What a page follows of a tree, and how a follow is brought current.
//!
//! A page reads a value by its route, then follows it by the version it holds
//! (`Follow`). Registering the follow and comparing the version happen in one
//! step of the tree, with nothing awaited between them, so no change falls
//! between the two:
//!
//! - the same version: nothing is sent, and a change after it is;
//! - the agents at an older revision of the same epoch: the entries changed
//!   since, as one `agents` frame;
//! - a transcript past the version held: the patches after it, from the
//!   recent patches of its node;
//! - a version of the store, which a page holds when it read a node no live
//!   session ran: compared with the store's now, which a read answers from and
//!   restores no tree for. When the tree restored from what the page read,
//!   and nothing changed since, the follow is answered `rebased`, which moves
//!   the page's copy to the live version without a read;
//! - any other version, or one the recent patches no longer reach: `stale`,
//!   and the follow ends.
//!
//! # Recent patches
//!
//! A node that runs keeps its latest patches in memory, as Kubernetes' watch
//! cache keeps a window of recent events: the patches of the last
//! [`RECENT_PATCHES`] operations at most and [`RECENT_BYTES`] bytes at most,
//! the oldest dropped first. They belong to the node's transcript and are
//! shared by every page that follows it, none for a page that left. The
//! buffer is made by the first patch a running node sends, and freed when
//! the node stops running or its tree is dropped. A node that does not run
//! changes by no patch, so it keeps none.
//!
//! The bound holds in the worst case: one buffer per running node, so a tree
//! holds at most as many mebibytes of recent patches as it has running nodes.

use std::collections::VecDeque;

use demi_agent_store::SavedVersions;
use demi_conversation_lane_protocol::{
    Followed, Held, STORED_EPOCH, ServerFrame, ShellStatus, TranscriptPatch, Version,
};
use demi_agent_tools::HostResolver;
use demi_shared_types::NodeId;

use super::Tree;
use crate::server::connection::Follows;

/// How many patch operations a node's recent patches hold at most.
///
/// A cache kept in memory for each running node: a page that falls behind it
/// reads again, so the bound loses nothing.
pub(crate) const RECENT_PATCHES: usize = 1000;
/// How many bytes, of the patches' JSON, a node's recent patches hold at most.
///
/// Bounded for the same reason as [`RECENT_PATCHES`].
pub(crate) const RECENT_BYTES: usize = 1 << 20;

/// One batch of patches, at the version after it.
struct Batch {
    version: Version,
    patches: Vec<TranscriptPatch>,
    operations: usize,
    bytes: usize,
}

/// A running node's latest patches, oldest first.
#[derive(Default)]
pub(crate) struct RecentPatches {
    batches: VecDeque<Batch>,
    operations: usize,
    bytes: usize,
}

impl RecentPatches {
    /// Keeps the batch `patches` at `version`, and drops the oldest until
    /// what is kept is within the bounds. A batch beyond them alone leaves
    /// nothing kept, so every follow before it is answered `stale`.
    pub(crate) fn keep(&mut self, version: Version, patches: &[TranscriptPatch]) {
        // A batch of another epoch than the last one's is of a transcript
        // that was built again: none before it leads to it.
        if self.batches.back().is_some_and(|last| last.version.epoch != version.epoch) {
            self.clear();
        }
        // The patches' JSON, which is what the pages are sent; a patch that
        // cannot be serialized is as large as any bound.
        let bytes = serde_json::to_vec(patches).map_or(usize::MAX, |json| json.len());
        self.operations += patches.len();
        self.bytes = self.bytes.saturating_add(bytes);
        self.batches.push_back(Batch {
            version,
            patches: patches.to_vec(),
            operations: patches.len(),
            bytes,
        });
        while self.operations > RECENT_PATCHES || self.bytes > RECENT_BYTES {
            let Some(dropped) = self.batches.pop_front() else {
                break;
            };
            self.operations -= dropped.operations;
            self.bytes = self.bytes.saturating_sub(dropped.bytes);
        }
    }

    fn clear(&mut self) {
        self.batches.clear();
        self.operations = 0;
        self.bytes = 0;
    }

    /// The batches after `held`, up to the version `current`, which lead from
    /// the one to the other without a gap; none when the recent patches do not
    /// reach `held`.
    fn after(&self, held: &Version, current: &Version) -> Option<Vec<&Batch>> {
        let mut next = held.revision + 1;
        let mut batches = Vec::new();
        for batch in self.batches.iter().filter(|batch| batch.version.revision > held.revision) {
            if batch.version.epoch != held.epoch || batch.version.revision != next {
                return None;
            }
            next += 1;
            batches.push(batch);
        }
        (next == current.revision + 1).then_some(batches)
    }
}

/// How a follow came out.
pub(crate) enum Following {
    /// The follow is registered; these frames bring the page current.
    Current(Vec<ServerFrame>),
    /// The page's version cannot be brought current: the follow ended.
    Stale,
}

/// What a follow of a value that no live session runs is answered: the
/// stored version the page holds is what the store holds now, or it is
/// stale.
pub(crate) fn compare_stored(value: &Followed, held: &Held, saved: Option<&SavedVersions>) -> Following {
    let (Held::Version { version, .. }, Some(saved)) = (held, saved) else {
        return Following::Stale;
    };
    let current = match value {
        Followed::Agents => Some(saved.agents),
        Followed::Transcript { node } => saved.nodes.get(node).copied(),
        _ => None,
    };
    if version.epoch == STORED_EPOCH && current == Some(version.revision) {
        Following::Current(Vec::new())
    } else {
        Following::Stale
    }
}

impl<H: HostResolver> Tree<H> {
    /// Brings a follow of `value` at the version `held` current for the
    /// connection whose follows are `follows`, which have the follow
    /// registered already, with nothing awaited between the registration and
    /// this call. `saved` is what the store holds, read before the step, for
    /// a page that holds what it read while no session lived.
    pub(crate) fn compare(&self, value: &Followed, held: &Held, saved: Option<&SavedVersions>) -> Following {
        match (value, held) {
            (Followed::Agents, Held::Version { version: held, .. }) => self.compare_agents(held, saved),
            (Followed::Transcript { node }, Held::Version { version: held, readings }) => {
                self.compare_transcript(node, held, *readings, saved)
            }
            // The backend's socket follows the other values itself, and a
            // value is followed by the kind of version it has.
            _ => Following::Stale,
        }
    }

    fn compare_agents(&self, held: &Version, saved: Option<&SavedVersions>) -> Following {
        let current = self.agents.version();
        let waits = |id: &NodeId| self.node(id).and_then(|node| node.session().waited_ms());
        let mut frames = Vec::new();
        if held.epoch == STORED_EPOCH {
            // The page read the agents while no session lived. The tree
            // restored from the store they were read from when the store
            // still holds that version: every entry is sent, and the page
            // holds them at the version the live agents have now.
            if saved.is_none_or(|saved| saved.agents != held.revision) {
                return Following::Stale;
            }
            frames.push(ServerFrame::Rebased {
                value: Followed::Agents,
                version: Version { epoch: current.epoch.clone(), revision: 0 },
            });
            frames.push(self.agents.since(0, waits));
        } else {
            if held.epoch != current.epoch || held.revision > current.revision {
                return Following::Stale;
            }
            if held.revision < current.revision {
                frames.push(self.agents.since(held.revision, waits));
            }
        }
        Following::Current(frames)
    }

    fn compare_transcript(&self, node: &NodeId, held: &Version, readings: bool, saved: Option<&SavedVersions>) -> Following {
        let Some(live) = self.node(node) else {
            // A node no live session runs does not change while it is not
            // run: a page holds what the store holds when its version is
            // the store's now.
            let following = compare_stored(
                &Followed::Transcript { node: node.clone() },
                &Held::Version { version: held.clone(), readings },
                saved,
            );
            return match following {
                Following::Current(_) => {
                    let mut frames = self.live_commands_of(node).into_iter().map(command_frame(node)).collect::<Vec<_>>();
                    if !readings {
                        frames.extend(self.agents.chart(node).map(|chart| ServerFrame::CacheChart { node: node.clone(), chart }));
                    }
                    Following::Current(frames)
                }
                stale => stale,
            };
        };
        let current = live.session().with_window(|transcript| transcript.version());
        let mut frames = Vec::new();
        if held.epoch == STORED_EPOCH {
            // The page read the node while no session lived: its session
            // restored from the store the page read when nothing was changed
            // since, so the page's copy is the live one.
            if current.revision != 0 || saved.is_none_or(|saved| saved.nodes.get(node) != Some(&held.revision)) {
                return Following::Stale;
            }
            frames.push(ServerFrame::Rebased {
                value: Followed::Transcript { node: node.clone() },
                version: current.clone(),
            });
        } else if held.epoch != current.epoch || held.revision > current.revision {
            return Following::Stale;
        } else if held.revision < current.revision {
            let recent = self.sink.recent.borrow();
            let Some(batches) = recent.get(node).and_then(|recent| recent.after(held, &current)) else {
                return Following::Stale;
            };
            frames.extend(batches.into_iter().map(|batch| ServerFrame::TranscriptPatch {
                node: node.clone(),
                patches: batch.patches.clone(),
                version: batch.version.clone(),
                failures: None,
                attachments: Vec::new(),
            }));
        }
        // A page that holds no reading, or was behind, may have missed the end
        // of a request and a call that landed, which no version covers: it is
        // sent the output and the chart as they stand.
        if !readings || held.revision < current.revision {
            frames.push(ServerFrame::RequestOutput {
                node: node.clone(),
                output: live.session().request_output(),
            });
            frames.extend(self.agents.chart(node).map(|chart| ServerFrame::CacheChart { node: node.clone(), chart }));
        }
        // The views of the node's live commands are no part of its version.
        frames.extend(self.live_commands_of(node).into_iter().map(command_frame(node)));
        Following::Current(frames)
    }

    /// The version of the transcript of the live node `node`, or none for a
    /// node no live session runs.
    pub fn transcript_version(&self, node: &NodeId) -> Option<Version> {
        let live = self.node(node)?;
        Some(live.session().with_window(|transcript| transcript.version()))
    }
}

/// The `shell_output` frame of `node`'s command `status`.
fn command_frame(node: &NodeId) -> impl Fn(ShellStatus) -> ServerFrame + '_ {
    move |status| ServerFrame::ShellOutput {
        node: node.clone(),
        status: Box::new(status),
    }
}

impl Follows {
    /// Registers the follow of `value`.
    pub(crate) fn register(&self, value: &Followed) {
        match value {
            Followed::Agents => self.follow_agents(),
            Followed::Transcript { node } => self.follow_node(node.clone()),
            Followed::Metadata | Followed::Saved | Followed::Workflows | Followed::Shares => {}
        }
    }

    /// Ends the follow of `value`.
    pub(crate) fn unregister(&self, value: &Followed) {
        match value {
            Followed::Agents => self.unfollow_agents(),
            Followed::Transcript { node } => self.unfollow_node(node),
            Followed::Metadata | Followed::Saved | Followed::Workflows | Followed::Shares => {}
        }
    }
}

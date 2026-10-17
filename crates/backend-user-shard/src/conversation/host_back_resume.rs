//! A root's turn its offline Host left (`host_offline`), resumed when that
//! device's runner connects again (`failures-and-recovery.md` § The
//! unfinished turn): the backend resumes it, as the user's Resume would and
//! with the reason that Resume carries, when the user's `resumeWhenHostBack`
//! preference says so at that moment and the conversation still runs on the
//! device.
//!
//! The shard follows each live root as it comes to rest, which its phase
//! change announces, and keeps for its conversation the `host_offline`
//! record that ended the root's last turn; the runner's hello is what says
//! the device is back, and a device whose runner is connected as the root
//! comes to rest came back while the turn ran. A record is resumed once: the turn the backend
//! resumed, left again by the same device, waits for the user's Resume, so a
//! device that keeps dropping costs one resumed turn. A tree evicted
//! meanwhile keeps its conversation's entry, and the resume opens it again.
//! The entries are the shard's alone: after a restart of the backend a
//! conversation has none until its tree is open again.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use demi_backend_host_access::root_of;
use demi_shared_types::{BlockId, ErrorBlock, HOST_OFFLINE, SessionPhase};
use demi_web_api_protocol::ids::{ConversationId, DeviceId};
use futures_util::future::join_all;

use crate::shard::Shard;

/// What each conversation's root waits for of a device that left its turn.
#[derive(Default)]
pub(crate) struct HostBackResumes(RefCell<HashMap<ConversationId, HostBack>>);

/// Where a root's turn its offline Host left stands.
enum HostBack {
    /// The root rests with the `host_offline` record `error`, which
    /// `device`'s next connection resumes.
    Waiting { error: BlockId, device: String },
    /// The backend took the record `error` up; until the root rests with
    /// another record, it waits for the user's Resume.
    Taken { error: BlockId },
    /// The backend resumed the turn the record `from` ended, which `device`
    /// left; the record the resumed turn ends with decides what follows.
    Resumed { from: BlockId, device: String },
}

/// The device whose absence the record `error` names, when it is a
/// `host_offline` record.
fn left_by(error: &ErrorBlock) -> Option<&str> {
    if error.code.as_deref() != Some(HOST_OFFLINE) {
        return None;
    }
    error.device.as_ref().map(|device| device.id.as_str())
}

impl HostBackResumes {
    /// Forgets every entry, for the shard's close.
    pub(crate) fn clear(&self) {
        self.0.borrow_mut().clear();
    }

    /// The conversations whose root waits for `device`, each with its
    /// record.
    fn waiting_for(&self, device: &str) -> Vec<(ConversationId, BlockId)> {
        self.0
            .borrow()
            .iter()
            .filter_map(|(conversation, entry)| match entry {
                HostBack::Waiting { error, device: away } if away == device => {
                    Some((conversation.clone(), error.clone()))
                }
                _ => None,
            })
            .collect()
    }

    /// Takes the record `error` of `conversation` up, once: whether it still
    /// waited, so that two connections of the device resume it once.
    fn take(&self, conversation: &ConversationId, error: &BlockId) -> bool {
        let mut entries = self.0.borrow_mut();
        let waits = matches!(
            entries.get(conversation),
            Some(HostBack::Waiting { error: waiting, .. }) if waiting == error
        );
        if waits {
            entries.insert(conversation.clone(), HostBack::Taken { error: error.clone() });
        }
        waits
    }
}

impl Shard {
    /// The conversation's root changed its phase, or its tree opened or
    /// went: a live root at rest whose last turn its offline Host left
    /// waits for that device, unless the backend resumed that very turn
    /// already; one whose last turn ended otherwise waits for none. A root
    /// that works, or a tree that went, leaves the entry as it is. A device
    /// that came back while the turn still ran is back already: the turn
    /// resumes as it comes to rest.
    pub(crate) fn follow_host_back(&self, conversation: &ConversationId) {
        let Some(device) = self.wait_for_host(conversation) else {
            return;
        };
        // The connection that binds after this read announces itself; the
        // record is taken once, whichever of the two comes first.
        match DeviceId::try_from(device.as_str()) {
            Ok(device) if self.devices().online(&device) => self.this().host_back(&device),
            Ok(_) => {}
            Err(error) => {
                tracing::warn!(%conversation, %device, "a turn names a device that is no device id: {error}");
            }
        }
    }

    /// Updates the conversation's entry as its root stands now, and answers
    /// the device the root newly waits for.
    fn wait_for_host(&self, conversation: &ConversationId) -> Option<String> {
        if self.is_closing() {
            return None;
        }
        let tree = self.agent().tree(&root_of(conversation))?;
        let session = tree.root().session();
        if session.phase() != SessionPhase::Idle {
            return None;
        }
        let failed = session.failed_turn();
        let mut entries = self.host_back_resumes().0.borrow_mut();
        let Some((error, device)) = failed
            .as_ref()
            .and_then(|failed| Some((failed.id.clone(), left_by(failed)?.to_owned())))
        else {
            entries.remove(conversation);
            return None;
        };
        let next = match entries.get(conversation) {
            Some(
                HostBack::Waiting { error: known, .. }
                | HostBack::Taken { error: known }
                | HostBack::Resumed { from: known, .. },
            ) if *known == error => return None,
            // The turn the backend resumed ended as the same device left it
            // again.
            Some(HostBack::Resumed { device: resumed, .. }) if *resumed == device => {
                entries.insert(conversation.clone(), HostBack::Taken { error });
                return None;
            }
            _ => HostBack::Waiting {
                error,
                device: device.clone(),
            },
        };
        entries.insert(conversation.clone(), next);
        Some(device)
    }

    /// The runner of `device` connected again: the turns its absence left
    /// resume, each in its own course.
    pub(crate) fn host_back(self: &Rc<Self>, device: &DeviceId) {
        let waiting = self.host_back_resumes().waiting_for(device.as_str());
        // Counted among the tree's openers, so the shard's close waits for
        // the resumes before the agent shuts down.
        let opening = self.tree_openers().token();
        let shard = self.clone();
        let device = device.clone();
        self.tasks().spawn_local(async move {
            {
                let _opening = opening;
                shard.resume_turns_left_by(&device, waiting).await;
            }
            #[cfg(feature = "testing")]
            shard
                .services()
                .hellos
                .pass(crate::holds::HelloStep::HostBack)
                .await;
        });
    }

    /// Resumes the turns `device` left, `waiting`, when the user's
    /// preference says so now.
    async fn resume_turns_left_by(&self, device: &DeviceId, waiting: Vec<(ConversationId, BlockId)>) {
        if waiting.is_empty() || self.is_closing() {
            return;
        }
        match self.services().control.preferences(self.user().clone()).await {
            Ok(preferences) if preferences.resume_when_host_back != Some(false) => {}
            Ok(_) => {
                tracing::info!(%device, "a device is back; the turns it left wait for the user, as their preference says");
                return;
            }
            Err(error) => {
                tracing::warn!(%device, "the preferences were not read for the turns a device left: {error}");
                return;
            }
        }
        join_all(
            waiting
                .into_iter()
                .map(|(conversation, error)| self.resume_turn_left_by(device, conversation, error)),
        )
        .await;
    }

    /// Resumes the conversation's turn that `device` left with the record
    /// `error`, while the conversation still runs on the device: one moved
    /// meanwhile waits for the user's Resume, which says where it now runs.
    async fn resume_turn_left_by(&self, device: &DeviceId, conversation: ConversationId, error: BlockId) {
        if self.is_closing() {
            return;
        }
        match self.placement(&conversation).await {
            Ok(placement) if placement.device.as_deref() == Some(device.as_str()) => {}
            Ok(_) => {
                tracing::info!(%conversation, %device, "a device is back, but the conversation it left moved; the turn waits for the user");
                return;
            }
            Err(why) => {
                tracing::warn!(%conversation, %device, "a turn a device left was not resumed: {why}");
                return;
            }
        }
        if !self.host_back_resumes().take(&conversation, &error) {
            return;
        }
        // No reason of its own: the user's Resume says the device is back.
        match self
            .resume_unfinished_turn(&conversation, |failed| failed.id == error, None)
            .await
        {
            Ok(true) => {
                self.host_back_resumes().0.borrow_mut().insert(
                    conversation.clone(),
                    HostBack::Resumed {
                        from: error,
                        device: device.to_string(),
                    },
                );
                tracing::info!(%conversation, %device, "a turn a device left resumed as it came back");
            }
            Ok(false) => {}
            Err(why) => {
                tracing::warn!(%conversation, %device, "a turn a device left was not resumed: {why}");
            }
        }
    }
}

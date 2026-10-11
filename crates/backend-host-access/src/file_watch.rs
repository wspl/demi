//! The file watch (`web-api.md` § File watch, `sessions-and-targets.md`
//! § Every way to a Host): a page's subscriptions to what it keeps of a
//! Host's files, and the Host's word that one of them changed. A
//! conversation's watch is admitted and ended as a user stream is, never
//! wakes a stopped Cloud, and is no activity; a device's folder dialog
//! watches through device access. The shard holds each page's
//! subscriptions on the Host, which pages that hold the same subject share
//! (`RemoteHost::subscriber`), and renews them while the page renews them.
//! The edge carries the messages, and holds the lease.
//!
//! What a page's subscriptions ask of the Host's, and the messages the page
//! hears, are `PageSubscriptions`', which a direct channel's watch on the
//! runner keeps too.

use std::time::Instant;

use demi_backend_remote_host::{HostEvent, HostSubscriber, RemoteHost};
use demi_host_interface::HostErrorKind;
use demi_runner_protocol::files::{Effect, PageSubscriptions};
use demi_web_api_protocol::files::{FileWatchMessage, FileWatchRequest};
use demi_web_api_protocol::ids::{ConversationId, DeviceId};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::HostShard;
use crate::access::{Attention, HostAccessError, Refusal};
use crate::lease::{Lease, Released};
use crate::transfer::OpenTransfer;

/// The page's messages that may wait for the shard.
const FROM_PAGE: usize = 8;
/// The messages for the page that may wait for the edge.
const TO_PAGE: usize = 64;

/// An open file watch, as the edge carries it. `Send`.
pub struct FileWatchChannel {
    /// What the page hears, in order; it ends when the Host does, after an
    /// `offline`.
    pub to_page: mpsc::Receiver<FileWatchMessage>,
    /// The page's messages.
    pub from_page: mpsc::Sender<FileWatchRequest>,
    pub lease: Lease,
}

/// What opening a file watch answers.
pub enum OpenedWatch {
    Watching(FileWatchChannel),
    /// The Host is out of reach or a stopped Cloud, which the page hears as
    /// `offline`.
    Offline,
}

impl dyn HostShard + '_ {
    /// Opens a page's file watch of the conversation's primary Host:
    /// admitted as a user stream is, without waking a stopped Cloud and
    /// without activity, and open until the edge drops its lease, the Host's
    /// connection ends, or a transition ends it.
    pub async fn open_file_watch(
        &self,
        id: &ConversationId,
        cancel: &CancellationToken,
    ) -> Result<OpenedWatch, HostAccessError> {
        let access = match self.admit_stream(id, Attention::Looks, cancel).await {
            Ok(access) => access,
            Err(HostAccessError::Refused(Refusal::Stopped)) => return Ok(OpenedWatch::Offline),
            Err(HostAccessError::Host(error)) if matches!(error.kind, HostErrorKind::Offline) => {
                return Ok(OpenedWatch::Offline);
            }
            Err(error) => return Err(error),
        };
        let root = access.host.root.clone();
        Ok(self.relay(&access.host.host, Some(root), access.open.ended.clone(), Some(access.open)))
    }

    /// Opens a device's folder dialog's file watch through device access
    /// (`sessions-and-targets.md` § Every way to a Host): every
    /// subscription of it is outside a working tree. A device whose runner
    /// is not connected is out of reach.
    pub fn open_device_watch(&self, device: &DeviceId) -> OpenedWatch {
        match self.devices().device_access(device) {
            Some(host) => self.relay(&host, None, CancellationToken::new(), None),
            None => OpenedWatch::Offline,
        }
    }

    /// Starts the relay of a page's subscriptions on `host`, whose
    /// conversation works in `root`, until the edge lets go or `ended`.
    fn relay(
        &self,
        host: &RemoteHost,
        root: Option<String>,
        ended: CancellationToken,
        open: Option<OpenTransfer>,
    ) -> OpenedWatch {
        let Ok(subscriber) = host.subscriber() else {
            return OpenedWatch::Offline;
        };
        let (to_page, page_hears) = mpsc::channel(TO_PAGE);
        let (page_says, from_page) = mpsc::channel(FROM_PAGE);
        let (lease, released) = Lease::new(ended.clone());
        let relay = Relay {
            subscriber,
            page: PageSubscriptions::new(root),
            to_page,
        };
        self.tasks()
            .spawn_local(relay.run(from_page, ended, released, open));
        OpenedWatch::Watching(FileWatchChannel {
            to_page: page_hears,
            from_page: page_says,
            lease,
        })
    }
}

struct Relay {
    subscriber: HostSubscriber,
    page: PageSubscriptions,
    to_page: mpsc::Sender<FileWatchMessage>,
}

/// Why the relay ended.
enum End {
    /// The edge let go, or a transition ended the watch.
    Released,
    /// The Host's connection ended: the page hears `offline`.
    Offline,
}

impl Relay {
    async fn run(
        mut self,
        mut from_page: mpsc::Receiver<FileWatchRequest>,
        ended: CancellationToken,
        released: Released,
        open: Option<OpenTransfer>,
    ) {
        tokio::pin!(released);
        let end = loop {
            let expiry = self.page.next_expiry();
            let effects = tokio::select! {
                () = &mut released => break End::Released,
                () = ended.cancelled() => break End::Released,
                request = from_page.recv() => {
                    let Some(request) = request else {
                        break End::Released;
                    };
                    // On the runtime's clock, which a test may hold still.
                    let now = tokio::time::Instant::now().into_std();
                    match request {
                        FileWatchRequest::Subscribe { subscriptions } => {
                            self.page.subscribe(subscriptions, now)
                        }
                        FileWatchRequest::Unsubscribe { keys } => self.page.unsubscribe(&keys),
                        FileWatchRequest::Renew { keys } => self.page.renew(&keys, now),
                    }
                }
                event = self.subscriber.next() => match event {
                    HostEvent::Live { subject, since, epoch } => self.page.live(&subject, since, &epoch),
                    HostEvent::Changed(changes) => self.page.changed(&changes),
                    HostEvent::Failed { subject, reason } => self.page.failed(&subject, &reason),
                    HostEvent::Ended => break End::Offline,
                },
                () = sleep_until(expiry), if expiry.is_some() => {
                    // The page stopped renewing these; it believes them over.
                    self.page.expire(tokio::time::Instant::now().into_std())
                }
            };
            if !self.apply(effects).await {
                break End::Released;
            }
        };
        if matches!(end, End::Offline) {
            // A page that went meanwhile hears nothing.
            let _ = self.to_page.send(FileWatchMessage::Offline).await;
        }
        // The subscriptions end with the relay, then its registration with
        // the conversation's transfers.
        drop(self);
        drop(open);
    }

    /// Carries out what the page's subscriptions say; false once the edge
    /// is gone.
    async fn apply(&self, effects: Vec<Effect>) -> bool {
        for effect in effects {
            match effect {
                Effect::Page(message) => {
                    if self.to_page.send(message).await.is_err() {
                        return false;
                    }
                }
                Effect::Hold(subject) => self.subscriber.hold(subject),
                Effect::Release(subject) => self.subscriber.release(&subject),
                Effect::Renew(subjects) => self.subscriber.renew(&subjects),
            }
        }
        true
    }
}

/// Waits until `deadline`, or forever without one.
async fn sleep_until(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline.into()).await,
        None => std::future::pending().await,
    }
}

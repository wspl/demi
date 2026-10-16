//! The file watch (`web-api-hosts.md` § File watch, `sessions-and-targets.md`
//! § Every way to a Host): a page's subscriptions to what it keeps of a
//! Host's files, and the Host's word that one of them changed. A
//! conversation's watch carries its primary Host's subscriptions and, under
//! a subscription's `host`, an attached host's; each host is admitted as a
//! user stream is when a subscription first names it, never wakes a stopped
//! Cloud, and is no activity. A device's folder dialog watches its one
//! device through device access. The shard holds each page's subscriptions
//! on each Host, which pages that hold the same subject share
//! (`RemoteHost::subscriber`), and renews them while the page renews them.
//! The edge carries the messages, and holds the lease.
//!
//! What a page's subscriptions ask of a Host's, and the messages the page
//! hears, are `PageSubscriptions`', which a direct channel's watch on the
//! runner keeps too.

use tracing::Instrument as _;
use std::rc::Rc;
use std::time::Instant;

use demi_backend_remote_host::{HostEvent, HostSubscriber};
use demi_host_interface::HostErrorKind;
use demi_runner_protocol::files::{Effect, PageSubscription, PageSubscriptions};
use demi_web_api_protocol::files::{
    ConversationSubscription, ConversationWatchRequest, FileWatchMessage,
};
use demi_web_api_protocol::ids::{ConversationId, DeviceId};
use futures_util::future::select_all;
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
    /// What the page hears, in order; it ends when a Host does, after an
    /// `offline`.
    pub to_page: mpsc::Receiver<FileWatchMessage>,
    /// The page's messages. A device's watch takes no subscription that
    /// names a host.
    pub from_page: mpsc::Sender<ConversationWatchRequest>,
    pub lease: Lease,
}

/// What opening a device's file watch answers.
pub enum OpenedWatch {
    Watching(FileWatchChannel),
    /// The device's runner is not connected, which the page hears as
    /// `offline`.
    Offline,
}

impl dyn HostShard + '_ {
    /// Opens a page's file watch of the conversation `id`, registered with
    /// its transfers so that a transition ends it, and open until the edge
    /// drops its lease, a Host's connection ends, or a transition ends it.
    /// Nothing is admitted yet: each Host is, without waking a stopped
    /// Cloud and without activity, when a subscription first names it, so a
    /// watch of an attached host does not hang on the primary Host.
    pub async fn open_file_watch(
        &self,
        id: &ConversationId,
    ) -> Result<FileWatchChannel, HostAccessError> {
        let record = self.owned_conversation(id).await?;
        if record.archived {
            return Err(Refusal::Archived.into());
        }
        let open = self
            .conversations()
            .slot(&record.id)
            .transfers
            .open()
            .map_err(|_| Refusal::Busy)?;
        let reach = Reach::Conversation {
            shard: self.this(),
            id: record.id,
        };
        Ok(self.relay(reach, Vec::new(), open.ended.clone(), Some(open)))
    }

    /// Opens a device's folder dialog's file watch through device access
    /// (`sessions-and-targets.md` § Every way to a Host): every
    /// subscription of it is outside a working tree. A device whose runner
    /// is not connected is out of reach.
    pub fn open_device_watch(&self, device: &DeviceId) -> OpenedWatch {
        let Some(subscriber) = self
            .devices()
            .device_access(device)
            .and_then(|host| host.subscriber().ok())
        else {
            return OpenedWatch::Offline;
        };
        let watch = HostWatch {
            host: None,
            subscriber,
            page: PageSubscriptions::new(None),
            _open: None,
        };
        OpenedWatch::Watching(self.relay(Reach::Device, vec![watch], CancellationToken::new(), None))
    }

    /// Starts the relay of a page's subscriptions, on `watches` and those
    /// `reach` admits, until the edge lets go or `ended`.
    fn relay(
        &self,
        reach: Reach,
        watches: Vec<HostWatch>,
        ended: CancellationToken,
        open: Option<OpenTransfer>,
    ) -> FileWatchChannel {
        let (to_page, page_hears) = mpsc::channel(TO_PAGE);
        let (page_says, from_page) = mpsc::channel(FROM_PAGE);
        let (lease, released) = Lease::new(ended.clone());
        let relay = Relay {
            reach,
            watches,
            to_page,
            ended,
        };
        // The watch logs under its socket's request ID.
        self.tasks()
            .spawn_local(relay.run(from_page, released, open).in_current_span());
        FileWatchChannel {
            to_page: page_hears,
            from_page: page_says,
            lease,
        }
    }
}

/// Where a relay's Hosts come from.
enum Reach {
    /// A device's folder dialog's: its one device, open from the start.
    Device,
    /// A conversation's: each Host admitted when a subscription first names
    /// it.
    Conversation {
        shard: Rc<dyn HostShard>,
        id: ConversationId,
    },
}

/// A page's subscriptions on one Host: the conversation's primary Host's
/// (`host` none), or an attached host's.
struct HostWatch {
    host: Option<DeviceId>,
    subscriber: HostSubscriber,
    page: PageSubscriptions,
    /// The admission's registration with the conversation's transfers,
    /// dropped after the subscriptions.
    _open: Option<OpenTransfer>,
}

struct Relay {
    reach: Reach,
    watches: Vec<HostWatch>,
    to_page: mpsc::Sender<FileWatchMessage>,
    /// Cancelled by a transition, or by the relay when a Host's admission
    /// says the conversation changed: the edge closes with
    /// `conversation_changed`.
    ended: CancellationToken,
}

/// Why the relay ended.
enum End {
    /// The edge let go, or a transition ended the watch.
    Released,
    /// A Host's connection ended, or it is out of reach or a stopped Cloud:
    /// the page hears `offline`.
    Offline,
    /// The conversation changed under an admission, as a transition would
    /// have ended the watch.
    Changed,
}

/// What the relay heard next.
enum Heard {
    Page(ConversationWatchRequest),
    Host(usize, HostEvent),
    Expired,
    Over(End),
}

impl Relay {
    async fn run(
        mut self,
        mut from_page: mpsc::Receiver<ConversationWatchRequest>,
        released: Released,
        open: Option<OpenTransfer>,
    ) {
        tokio::pin!(released);
        let end = loop {
            let expiry = self.watches.iter().filter_map(|watch| watch.page.next_expiry()).min();
            let heard = tokio::select! {
                () = &mut released => Heard::Over(End::Released),
                () = self.ended.cancelled() => Heard::Over(End::Released),
                request = from_page.recv() => match request {
                    Some(request) => Heard::Page(request),
                    None => Heard::Over(End::Released),
                },
                (index, event) = next_event(&mut self.watches) => Heard::Host(index, event),
                () = sleep_until(expiry), if expiry.is_some() => Heard::Expired,
            };
            // On the runtime's clock, which a test may hold still.
            let now = tokio::time::Instant::now().into_std();
            let outcome = match heard {
                Heard::Over(end) => break end,
                Heard::Page(ConversationWatchRequest::Subscribe { subscriptions }) => {
                    // An admission ends with the watch: a transition's end
                    // cancels its waits, and the edge's here.
                    let admitting = tokio::select! {
                        () = &mut released => Err(End::Released),
                        subscribed = self.subscribe(subscriptions, now) => subscribed,
                    };
                    match admitting {
                        Ok(applied) => applied,
                        Err(end) => break end,
                    }
                }
                Heard::Page(ConversationWatchRequest::Unsubscribe { keys }) => {
                    self.each(|page| page.unsubscribe(&keys)).await
                }
                Heard::Page(ConversationWatchRequest::Renew { keys }) => self.renew(&keys, now).await,
                Heard::Host(index, event) => {
                    let watch = &mut self.watches[index];
                    let effects = match event {
                        HostEvent::Live { subject, since, epoch } => watch.page.live(&subject, since, &epoch),
                        HostEvent::Changed(changes) => watch.page.changed(&changes),
                        HostEvent::Failed { subject, reason } => watch.page.failed(&subject, &reason),
                        HostEvent::Ended => break End::Offline,
                    };
                    self.apply(index, effects).await
                }
                // The page stopped renewing these; it believes them over.
                Heard::Expired => self.each(|page| page.expire(now)).await,
            };
            if !outcome {
                break End::Released;
            }
        };
        match end {
            // A page that went meanwhile hears nothing.
            End::Offline => drop(self.to_page.send(FileWatchMessage::Offline).await),
            End::Changed => self.ended.cancel(),
            End::Released => {}
        }
        // The subscriptions end with the relay, then the registrations with
        // the conversation's transfers.
        drop(self);
        drop(open);
    }

    /// The page subscribes: each subscription on the Host it names, which a
    /// conversation's watch admits first when none of the page's
    /// subscriptions is on it yet. A key the page moves to another Host
    /// lets go of the one it was on. False once the edge is gone.
    async fn subscribe(
        &mut self,
        subscriptions: Vec<ConversationSubscription>,
        now: Instant,
    ) -> Result<bool, End> {
        let mut by_host: Vec<(Option<DeviceId>, Vec<PageSubscription>)> = Vec::new();
        for subscription in subscriptions {
            let (host, subscription) = subscription.into_parts();
            match by_host.iter_mut().find(|(named, _)| *named == host) {
                Some((_, group)) => group.push(subscription),
                None => by_host.push((host, vec![subscription])),
            }
        }
        for (host, group) in by_host {
            let keys: Vec<String> = group.iter().map(|subscription| subscription.key.clone()).collect();
            for index in 0..self.watches.len() {
                if self.watches[index].host != host {
                    let effects = self.watches[index].page.unsubscribe(&keys);
                    if !self.apply(index, effects).await {
                        return Ok(false);
                    }
                }
            }
            let index = match self.watches.iter().position(|watch| watch.host == host) {
                Some(index) => index,
                None => match self.admit(host.as_ref()).await {
                    Admission::Watching(watch) => {
                        self.watches.push(watch);
                        self.watches.len() - 1
                    }
                    Admission::Unavailable(reason) => {
                        for key in keys {
                            let message = FileWatchMessage::Unavailable {
                                key,
                                reason: reason.clone(),
                            };
                            if self.to_page.send(message).await.is_err() {
                                return Ok(false);
                            }
                        }
                        continue;
                    }
                    Admission::Over(end) => return Err(end),
                },
            };
            let effects = self.watches[index].page.subscribe(group, now);
            if !self.apply(index, effects).await {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Admits the conversation's Host `host` names, as a user stream is: a
    /// stopped Cloud is not woken, and a Host out of reach is offline.
    async fn admit(&self, host: Option<&DeviceId>) -> Admission {
        let Reach::Conversation { shard, id } = &self.reach else {
            // The device's watch reads only what names no host.
            return Admission::Unavailable("This watch is of one device.".to_owned());
        };
        let cancel = self.ended.child_token();
        let access = match shard.admit_stream(id, host, Attention::Looks, &cancel).await {
            Ok(access) => access,
            Err(HostAccessError::Refused(Refusal::Stopped)) => return Admission::Over(End::Offline),
            Err(HostAccessError::Host(error)) if matches!(error.kind, HostErrorKind::Offline) => {
                return Admission::Over(End::Offline);
            }
            Err(
                HostAccessError::Missing
                | HostAccessError::Refused(Refusal::Archived | Refusal::Busy),
            ) => return Admission::Over(End::Changed),
            Err(HostAccessError::Cancelled) => return Admission::Over(End::Released),
            Err(error) => return Admission::Unavailable(error.to_string()),
        };
        let Ok(subscriber) = access.host.host.subscriber() else {
            return Admission::Over(End::Offline);
        };
        // Only the primary Host has the conversation's working tree; all of
        // an attached host's subscriptions are outside it.
        let root = access.primary.then(|| access.host.root.clone());
        Admission::Watching(HostWatch {
            host: host.cloned(),
            subscriber,
            page: PageSubscriptions::new(root),
            _open: Some(access.open),
        })
    }

    /// The page renews `keys`, on whichever Host each is, which it hears in
    /// one `renewed`, as it counts one answer for each `renew`.
    async fn renew(&mut self, keys: &[String], now: Instant) -> bool {
        let mut renewed = Vec::new();
        for index in 0..self.watches.len() {
            let mut effects = self.watches[index].page.renew(keys, now);
            effects.retain(|effect| match effect {
                Effect::Page(FileWatchMessage::Renewed { keys }) => {
                    renewed.extend(keys.iter().cloned());
                    false
                }
                _ => true,
            });
            if !self.apply(index, effects).await {
                return false;
            }
        }
        let message = FileWatchMessage::Renewed { keys: renewed };
        self.to_page.send(message).await.is_ok()
    }

    /// Does `change` to the subscriptions on every Host; false once the
    /// edge is gone.
    async fn each(&mut self, mut change: impl FnMut(&mut PageSubscriptions) -> Vec<Effect>) -> bool {
        for index in 0..self.watches.len() {
            let effects = change(&mut self.watches[index].page);
            if !self.apply(index, effects).await {
                return false;
            }
        }
        true
    }

    /// Carries out what the page's subscriptions on the Host of `index`
    /// say; false once the edge is gone.
    async fn apply(&self, index: usize, effects: Vec<Effect>) -> bool {
        let subscriber = &self.watches[index].subscriber;
        for effect in effects {
            match effect {
                Effect::Page(message) => {
                    if self.to_page.send(message).await.is_err() {
                        return false;
                    }
                }
                Effect::Hold(subject) => subscriber.hold(subject),
                Effect::Release(subject) => subscriber.release(&subject),
                Effect::Renew(subjects) => subscriber.renew(&subjects),
            }
        }
        true
    }
}

/// What admitting a Host for a subscription answers.
enum Admission {
    Watching(HostWatch),
    /// The Host cannot be watched for this reason, such as a device that is
    /// not attached: the subscriptions that name it are over.
    Unavailable(String),
    /// The watch ends.
    Over(End),
}

/// What the first of the Hosts reports, with its index; forever without
/// one.
async fn next_event(watches: &mut [HostWatch]) -> (usize, HostEvent) {
    if watches.is_empty() {
        return std::future::pending().await;
    }
    let (event, index, _) = select_all(
        watches
            .iter_mut()
            .map(|watch| Box::pin(watch.subscriber.next())),
    )
    .await;
    (index, event)
}

/// Waits until `deadline`, or forever without one.
async fn sleep_until(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline.into()).await,
        None => std::future::pending().await,
    }
}

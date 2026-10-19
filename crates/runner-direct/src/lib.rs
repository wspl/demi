//! The runner's side of direct channels (`direct-channel.md`), with a page
//! or with another runner (`port-forwarding.md`): the peers the backend
//! introduces, offering or answering, their UDP sockets on `127.0.0.1` and
//! the local network's addresses, ICE, DTLS and SCTP through str0m, and
//! each channel's work: a page's operation, whose header and answer this
//! crate handles and which the runner carries out through [`Operations`],
//! or a forwarded TCP connection, a byte stream both ways
//! ([`ForwardChannel`]). Every channel goes through one flow control.
//!
//! The peers run on a thread of their own, off the runner's control
//! thread, since a peer at full speed fills a core: the runner drives them
//! through [`Direct`], and runs [`DirectDriver`] on that thread. A pair of
//! runners keeps its peer as [`Pairs`] says: made at the pair's first
//! connection, closed a minute after its last, and tried again on the
//! direct channel's schedule while the pair is on the relay.

mod addresses;
mod channel;
mod forward;
mod gathering;
mod operation;
mod pairs;
mod peer;
#[cfg(feature = "testing")]
pub mod testing;

use std::collections::HashMap;
use std::sync::Arc;

use demi_runner_protocol::direct::{Introduced, MAX_PEERS, OfferRefusal, StunUrl};
use demi_runner_protocol::forward::PairReason;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

pub use addresses::{Addresses, offered};
pub use forward::{ForwardChannel, Forwarded, connect_target, give_up_on};
pub use operation::{
    Answer, ByteStream, FileRange, FileText, Listing, Operations, Scope, StreamActivity,
    StreamRequest, WatchStream,
};
pub use pairs::{Dial, Opened, PairPeer, Pairs, Path, PathGuard};
pub use peer::PeerStatus;

use peer::{Behaviour, LocalOpen, OfferError, Peer, Remote, Signals};

/// The runner's hold on its peers. Dropping it ends the driver, and with it
/// every peer.
#[derive(Clone)]
pub struct Direct {
    commands: mpsc::UnboundedSender<Command>,
    peers: watch::Receiver<usize>,
}

/// A runner's answer to an offer: its SDP, and the candidates it finds
/// after, such as the address a STUN server saw it at, until the peer ends.
#[derive(Debug)]
pub struct Answered {
    pub sdp: String,
    pub candidates: mpsc::UnboundedReceiver<String>,
}

/// A runner's offer to another runner: its SDP, the candidates it finds
/// after, and the peer, which connects once the other runner's answer
/// arrives ([`Direct::answered`]).
pub struct Dialing {
    pub sdp: String,
    pub candidates: mpsc::UnboundedReceiver<String>,
    pub link: PeerLink,
}

/// What a runner says to an offer it does not answer, or why it made no
/// offer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused {
    pub code: OfferRefusal,
    pub message: String,
}

/// A peer this runner offered: it opens the peer's channels, and hears
/// where the peer is.
#[derive(Clone)]
pub struct PeerLink {
    local: mpsc::UnboundedSender<LocalOpen>,
    status: watch::Receiver<PeerStatus>,
}

impl PeerLink {
    /// Where the peer is now, and from now on.
    pub fn status(&self) -> watch::Receiver<PeerStatus> {
        self.status.clone()
    }

    /// Waits until the peer connected, or answers why it ended first.
    pub async fn connected(&self) -> Result<(), Option<PairReason>> {
        let mut status = self.status.clone();
        let seen = status
            .wait_for(|status| *status != PeerStatus::Connecting)
            .await
            .map(|status| status.clone())
            .unwrap_or(PeerStatus::Ended(None));
        match seen {
            PeerStatus::Connected => Ok(()),
            PeerStatus::Connecting => unreachable!("the wait passed connecting"),
            PeerStatus::Ended(reason) => Err(reason),
        }
    }

    /// Waits until the peer ended, and answers why.
    pub async fn ended(&self) -> Option<PairReason> {
        let mut status = self.status.clone();
        let seen = status
            .wait_for(|status| matches!(status, PeerStatus::Ended(_)))
            .await
            .map(|status| status.clone());
        match seen {
            Ok(PeerStatus::Ended(reason)) => reason,
            // A peer whose loop is gone ended with it.
            _ => None,
        }
    }

    /// Opens a channel for a connection of forward `forward`; none once the
    /// peer ended.
    pub async fn open(&self, forward: &str) -> Option<ForwardChannel> {
        let (open, named, input, output, cancel, abort) = peer::forward_open(forward);
        self.local.send(open).ok()?;
        let (id, sender) = named.await.ok()?;
        Some(ForwardChannel {
            input,
            output,
            took: forward::Took { id, sender },
            cancel,
            abort,
        })
    }
}

enum Command {
    Offer {
        peer: String,
        sdp: String,
        introduced: Introduced,
        stun: Vec<StunUrl>,
        answer: oneshot::Sender<Result<Answered, Refused>>,
    },
    Dial {
        peer: String,
        stun: Vec<StunUrl>,
        dialing: oneshot::Sender<Result<Dialing, Refused>>,
    },
    Remote {
        peer: String,
        said: Remote,
    },
    Close {
        peer: String,
    },
    CloseAll,
}

/// Serves the peers on the thread it runs on, until [`Direct`] is dropped.
pub struct DirectDriver {
    commands: mpsc::UnboundedReceiver<Command>,
    peers: watch::Sender<usize>,
    operations: Arc<dyn Operations>,
    addresses: Addresses,
}

/// A runner's peers, which carry out their operations through
/// `operations`, with sockets on `addresses`.
pub fn direct(operations: Arc<dyn Operations>, addresses: Addresses) -> (Direct, DirectDriver) {
    let (commands, received) = mpsc::unbounded_channel();
    let (peers, counted) = watch::channel(0);
    let direct = Direct {
        commands,
        peers: counted,
    };
    let driver = DirectDriver {
        commands: received,
        peers,
        operations,
        addresses,
    };
    (direct, driver)
}

/// The refusal of a driver that ended.
fn gone() -> Refused {
    Refused {
        code: OfferRefusal::Busy,
        message: "the runner's peers have stopped".into(),
    }
}

impl Direct {
    /// Answers the offer of whom the backend introduced, a page or another
    /// runner, for peer `peer` with a new peer, which replaces the peer of
    /// that id and asks the STUN servers `stun` for its public addresses.
    pub async fn offer(
        &self,
        peer: String,
        sdp: String,
        introduced: Introduced,
        stun: Vec<StunUrl>,
    ) -> Result<Answered, Refused> {
        let (answer, answered) = oneshot::channel();
        let command = Command::Offer {
            peer,
            sdp,
            introduced,
            stun,
            answer,
        };
        self.commands.send(command).map_err(|_| gone())?;
        answered.await.map_err(|_| gone())?
    }

    /// Offers a new peer `peer` to another runner, which asks the STUN
    /// servers `stun` for its public addresses; the other runner's answer
    /// arrives through [`Direct::answered`].
    pub async fn dial(&self, peer: String, stun: Vec<StunUrl>) -> Result<Dialing, Refused> {
        let (dialing, dialed) = oneshot::channel();
        let command = Command::Dial { peer, stun, dialing };
        self.commands.send(command).map_err(|_| gone())?;
        dialed.await.map_err(|_| gone())?
    }

    /// The other runner's answer to peer `peer`'s offer.
    pub fn answered(&self, peer: &str, sdp: String) {
        // A driver that ended has no peer left to answer.
        let _ = self.commands.send(Command::Remote {
            peer: peer.into(),
            said: Remote::Answer(sdp),
        });
    }

    /// A candidate the other side of peer `peer` found after its offer or
    /// answer; one for a peer the runner no longer has is passed over.
    pub fn candidate(&self, peer: &str, candidate: String) {
        // A driver that ended has no peer left to check it.
        let _ = self.commands.send(Command::Remote {
            peer: peer.into(),
            said: Remote::Candidate(candidate),
        });
    }

    /// Closes peer `peer`, whose page's signaling socket closed, or whose
    /// other runner's side ended.
    pub fn close(&self, peer: &str) {
        // A driver that ended has no peer left to close.
        let _ = self.commands.send(Command::Close { peer: peer.into() });
    }

    /// Closes every peer: the runner's connection to the backend ended, so
    /// it can no longer hear that the other side went away.
    pub fn close_all(&self) {
        // A driver that ended has no peer left to close.
        let _ = self.commands.send(Command::CloseAll);
    }

    /// How many peers the runner keeps, from now on.
    pub fn peers(&self) -> watch::Receiver<usize> {
        self.peers.clone()
    }
}

/// A served peer: its generation, since a new offer replaces it under the
/// same id, what closes it, and where the other side's answer and later
/// candidates go.
struct Served {
    generation: u64,
    cancel: CancellationToken,
    remote: mpsc::UnboundedSender<Remote>,
}

impl DirectDriver {
    /// Runs the driver on a thread of its own with a runtime of its own;
    /// the thread ends when [`Direct`] is dropped.
    pub fn spawn(self) -> std::io::Result<std::thread::JoinHandle<()>> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        std::thread::Builder::new()
            .name("direct-peers".into())
            .spawn(move || runtime.block_on(self.run()))
    }

    /// Serves offers and closes until [`Direct`] is dropped, then closes
    /// every peer and waits for them to end.
    pub async fn run(mut self) {
        let provider = Arc::new(str0m::crypto::from_feature_flags());
        let mut served: HashMap<String, Served> = HashMap::new();
        let mut tasks: JoinSet<(String, u64)> = JoinSet::new();
        let mut generation = 0;
        loop {
            tokio::select! {
                command = self.commands.recv() => match command {
                    Some(Command::Offer { peer, sdp, introduced, stun, answer }) => {
                        // A new offer replaces the peer of its id.
                        if let Some(replaced) = served.remove(&peer) {
                            replaced.cancel.cancel();
                        }
                        if served.len() >= MAX_PEERS {
                            // The side that asked may have gone.
                            let _ = answer.send(Err(Refused {
                                code: OfferRefusal::Busy,
                                message: format!("the runner keeps {MAX_PEERS} peers"),
                            }));
                            continue;
                        }
                        let behaviour = match introduced {
                            Introduced::Page { page, live } => Behaviour::Page {
                                operations: self.operations.clone(),
                                introduction: Arc::new(page),
                                live,
                            },
                            Introduced::Runner { device } => Behaviour::Runner {
                                operations: self.operations.clone(),
                                device,
                            },
                        };
                        let made = Peer::answer(&sdp, self.addresses.current(), provider.clone(), behaviour, stun).await;
                        let (made, sdp) = match made {
                            Ok(made) => made,
                            Err(error) => {
                                // The side that asked may have gone.
                                let _ = answer.send(Err(refusal(error)));
                                continue;
                            }
                        };
                        generation += 1;
                        // An answering peer opens no channel of its own.
                        let (candidates, _, _) = self.serve(&mut served, &mut tasks, peer, made, generation);
                        // A side that went before its answer has a peer
                        // that gives up.
                        let _ = answer.send(Ok(Answered { sdp, candidates }));
                    }
                    Some(Command::Dial { peer, stun, dialing }) => {
                        if let Some(replaced) = served.remove(&peer) {
                            replaced.cancel.cancel();
                        }
                        if served.len() >= MAX_PEERS {
                            // The runner that asked may have gone.
                            let _ = dialing.send(Err(Refused {
                                code: OfferRefusal::Busy,
                                message: format!("the runner keeps {MAX_PEERS} peers"),
                            }));
                            continue;
                        }
                        let (made, sdp) = match Peer::offer(self.addresses.current(), provider.clone(), stun).await {
                            Ok(made) => made,
                            Err(error) => {
                                // The runner that asked may have gone.
                                let _ = dialing.send(Err(refusal(error)));
                                continue;
                            }
                        };
                        generation += 1;
                        let (candidates, local, status) = self.serve(&mut served, &mut tasks, peer, made, generation);
                        // The runner that asked may have gone; its peer
                        // then gives up unanswered.
                        let _ = dialing.send(Ok(Dialing { sdp, candidates, link: PeerLink { local, status } }));
                    }
                    Some(Command::Remote { peer, said }) => {
                        if let Some(served) = served.get(&peer) {
                            // A peer that ended takes nothing.
                            let _ = served.remote.send(said);
                        }
                    }
                    Some(Command::Close { peer }) => {
                        if let Some(closed) = served.remove(&peer) {
                            closed.cancel.cancel();
                        }
                    }
                    Some(Command::CloseAll) => {
                        for (_, closed) in served.drain() {
                            closed.cancel.cancel();
                        }
                    }
                    None => break,
                },
                Some(ended) = tasks.join_next() => {
                    if let Ok((peer, ended)) = ended
                        && served.get(&peer).is_some_and(|peer| peer.generation == ended)
                    {
                        served.remove(&peer);
                    }
                }
            }
            self.peers.send_replace(served.len());
        }
        for (_, closed) in served.drain() {
            closed.cancel.cancel();
        }
        while tasks.join_next().await.is_some() {}
        self.peers.send_replace(0);
    }

    /// Serves `made` as peer `peer` of `generation`: the candidates it finds
    /// after its offer or answer, where its channels are opened, and where
    /// it is.
    fn serve(
        &self,
        served: &mut HashMap<String, Served>,
        tasks: &mut JoinSet<(String, u64)>,
        peer: String,
        made: Peer,
        generation: u64,
    ) -> (
        mpsc::UnboundedReceiver<String>,
        mpsc::UnboundedSender<LocalOpen>,
        watch::Receiver<PeerStatus>,
    ) {
        let cancel = CancellationToken::new();
        let (remote, heard) = mpsc::unbounded_channel();
        let (found, candidates) = mpsc::unbounded_channel();
        let (local, opened) = mpsc::unbounded_channel();
        let (status, watching) = watch::channel(PeerStatus::Connecting);
        served.insert(
            peer.clone(),
            Served {
                generation,
                cancel: cancel.clone(),
                remote,
            },
        );
        let signals = Signals {
            remote: heard,
            found,
            local: opened,
            status,
        };
        tasks.spawn(async move {
            made.serve(cancel, signals).await;
            (peer, generation)
        });
        (candidates, local, watching)
    }
}

/// What the other side hears when no peer was made.
fn refusal(error: OfferError) -> Refused {
    let code = match error {
        OfferError::Invalid(_) => OfferRefusal::InvalidOffer,
        OfferError::Sockets(_) => OfferRefusal::Busy,
    };
    Refused {
        code,
        message: error.to_string(),
    }
}

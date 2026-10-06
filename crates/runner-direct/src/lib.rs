//! The runner's side of direct channels (`direct-channel.md`): the peers
//! the backend introduces, their UDP sockets on `127.0.0.1` and the local
//! network's addresses, ICE, DTLS and SCTP through str0m, and each
//! channel's operation, whose header, answer and flow control this crate
//! handles and which the runner carries out through [`Operations`].
//!
//! The peers run on a thread of their own, off the runner's control
//! thread, since a peer at full speed fills a core: the runner drives them
//! through [`Direct`], and runs [`DirectDriver`] on that thread.

mod addresses;
mod operation;
mod peer;

use std::collections::HashMap;
use std::sync::Arc;

use demi_runner_protocol::direct::{Introduction, MAX_PEERS, OfferRefusal};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

pub use addresses::{Addresses, offered};
pub use operation::{
    Answer, ByteStream, FileRange, FileText, Listing, Operations, Scope, StreamRequest, WatchStream,
};

use peer::{OfferError, Peer};

/// The runner's hold on its peers. Dropping it ends the driver, and with it
/// every peer.
#[derive(Clone)]
pub struct Direct {
    commands: mpsc::UnboundedSender<Command>,
    peers: watch::Receiver<usize>,
}

/// What a runner says to an offer it does not answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused {
    pub code: OfferRefusal,
    pub message: String,
}

enum Command {
    Offer {
        peer: String,
        sdp: String,
        introduction: Introduction,
        answer: oneshot::Sender<Result<String, Refused>>,
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

impl Direct {
    /// Answers a page's offer for peer `peer` with a new peer, which
    /// replaces the peer of that id.
    pub async fn offer(
        &self,
        peer: String,
        sdp: String,
        introduction: Introduction,
    ) -> Result<String, Refused> {
        let (answer, answered) = oneshot::channel();
        let command = Command::Offer {
            peer,
            sdp,
            introduction,
            answer,
        };
        let gone = || Refused {
            code: OfferRefusal::Busy,
            message: "the runner's peers have stopped".into(),
        };
        self.commands.send(command).map_err(|_| gone())?;
        answered.await.map_err(|_| gone())?
    }

    /// Closes peer `peer`, whose page's signaling socket closed.
    pub fn close(&self, peer: &str) {
        // A driver that ended has no peer left to close.
        let _ = self.commands.send(Command::Close { peer: peer.into() });
    }

    /// Closes every peer: the runner's connection to the backend ended, so
    /// it can no longer hear that a page went away.
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
/// same id, and what closes it.
struct Served {
    generation: u64,
    cancel: CancellationToken,
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
                    Some(Command::Offer { peer, sdp, introduction, answer }) => {
                        // A new offer replaces the peer of its id.
                        if let Some(replaced) = served.remove(&peer) {
                            replaced.cancel.cancel();
                        }
                        if served.len() >= MAX_PEERS {
                            // The page that asked may have gone.
                            let _ = answer.send(Err(Refused {
                                code: OfferRefusal::Busy,
                                message: format!("the runner keeps {MAX_PEERS} peers"),
                            }));
                            continue;
                        }
                        let made = Peer::answer(
                            &sdp,
                            self.addresses.current(),
                            provider.clone(),
                            self.operations.clone(),
                            Arc::new(introduction),
                        )
                        .await;
                        let (made, sdp) = match made {
                            Ok(made) => made,
                            Err(error) => {
                                let code = match error {
                                    OfferError::Invalid(_) => OfferRefusal::InvalidOffer,
                                    OfferError::Sockets(_) => OfferRefusal::Busy,
                                };
                                let message = error.to_string();
                                // The page that asked may have gone.
                                let _ = answer.send(Err(Refused { code, message }));
                                continue;
                            }
                        };
                        generation += 1;
                        let cancel = CancellationToken::new();
                        served.insert(peer.clone(), Served { generation, cancel: cancel.clone() });
                        let current = generation;
                        tasks.spawn(async move {
                            made.serve(cancel).await;
                            (peer, current)
                        });
                        // A page that went before its answer has a peer
                        // that gives up.
                        let _ = answer.send(Ok(sdp));
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
}

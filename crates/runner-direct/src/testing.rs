//! A page in process (`testing`): the runner's own offering peer on
//! `127.0.0.1`, which connects to a runner's peers as a browser does and
//! speaks the channels' protocol as a page does, its offer delivered to the
//! runner directly or through the backend's signaling.

use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use demi_runner_protocol::direct::{Introduced, Introduction};
use str0m::channel::ChannelConfig;
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;
use tokio_util::task::AbortOnDropHandle;

use crate::channel::Kind;
use crate::operation::{Input, OUTPUT_MESSAGES, Output};
use crate::peer::{LocalOpen, Peer, PeerStatus, Remote, Signals, probe_config};
use crate::{Direct, Refused};

/// A message the runner sent on a channel, or its close.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Heard {
    Text(String),
    Binary(Bytes),
    Closed,
}

impl Heard {
    pub fn json(&self) -> serde_json::Value {
        match self {
            Self::Text(text) => serde_json::from_str(text).expect("the runner sends JSON text"),
            other => panic!("expected JSON text, heard {other:?}"),
        }
    }
}

/// The page's peer.
pub struct Page {
    local: mpsc::UnboundedSender<LocalOpen>,
    /// Whether the connection stands: true once it connected, false once it
    /// failed or closed.
    pub connected: watch::Receiver<Option<bool>>,
    _task: AbortOnDropHandle<()>,
    _status: AbortOnDropHandle<()>,
}

/// A channel of the page's.
pub struct PageChannel {
    sending: Mutex<Option<mpsc::UnboundedSender<Output>>>,
    heard: mpsc::UnboundedReceiver<Input>,
}

/// An offer's SDP without its candidates, and its first candidate as a
/// browser trickles it.
fn without_candidates(sdp: &str) -> (String, Option<String>) {
    let mut first = None;
    let kept = sdp
        .lines()
        .filter(|line| match line.strip_prefix("a=") {
            Some(candidate) if candidate.starts_with("candidate") => {
                first.get_or_insert_with(|| candidate.to_owned());
                false
            }
            _ => true,
        })
        .map(|line| format!("{line}\r\n"))
        .collect();
    (kept, first)
}

impl Page {
    /// Offers a peer `peer` to `direct`, as the backend's introduction
    /// carries it, and serves the connection the answer makes.
    pub async fn offer(direct: &Direct, peer: &str, introduction: Introduction) -> Result<Self, Refused> {
        Self::offer_as(direct, peer, introduction, false).await
    }

    /// Offers the page's peer for live views (`crates/runner-direct/src/lib.rs`).
    pub async fn offer_live(direct: &Direct, peer: &str, introduction: Introduction) -> Result<Self, Refused> {
        Self::offer_as(direct, peer, introduction, true).await
    }

    async fn offer_as(direct: &Direct, peer: &str, introduction: Introduction, live: bool) -> Result<Self, Refused> {
        Self::connect(async |offer| {
            let introduced = Introduced::Page { page: introduction, live };
            let answered = direct.offer(peer.into(), offer, introduced, Vec::new()).await?;
            Ok(answered.sdp)
        })
        .await
    }

    /// Offers a peer `peer` to `direct` with no candidate, and reads the
    /// runner's answer without its candidates, so that neither end can
    /// reach the other until the page trickles its own after the answer, as
    /// a browser does: the runner's checks to it then make the connection.
    pub async fn trickled(direct: &Direct, peer: &str, introduction: Introduction) -> Result<Self, Refused> {
        let mut trickled = None;
        let page = Self::make(async |offer| {
            let (offer, candidate) = without_candidates(&offer);
            trickled = candidate;
            let introduced = Introduced::Page { page: introduction, live: false };
            let answered = direct.offer(peer.into(), offer, introduced, Vec::new()).await?;
            Ok(without_candidates(&answered.sdp).0)
        })
        .await?;
        direct.candidate(peer, trickled.expect("the page's offer names its address"));
        Ok(page)
    }

    /// Makes the page's offer, which `answer` takes to the runner and whose
    /// answer it gives back, as the backend's signaling does, and serves the
    /// connection the answer makes.
    pub async fn connect<E>(answer: impl AsyncFnOnce(String) -> Result<String, E>) -> Result<Self, E> {
        Self::make(answer).await
    }

    async fn make<E>(answer: impl AsyncFnOnce(String) -> Result<String, E>) -> Result<Self, E> {
        let provider = Arc::new(str0m::crypto::from_feature_flags());
        let (made, offer) = Peer::offer(vec![Ipv4Addr::LOCALHOST], provider, Vec::new())
            .await
            .expect("the page binds 127.0.0.1");
        let answer = answer(offer).await?;
        let (remote, heard) = mpsc::unbounded_channel();
        // The answer goes ahead of every candidate the test trickles.
        remote.send(Remote::Answer(answer)).expect("the peer is not served yet");
        let (found, _found) = mpsc::unbounded_channel();
        let (local, opened) = mpsc::unbounded_channel();
        let (status, mut watching) = watch::channel(PeerStatus::Connecting);
        let signals = Signals {
            remote: heard,
            found,
            local: opened,
            status,
        };
        let cancel = CancellationToken::new();
        let task = AbortOnDropHandle::new(tokio::spawn(async move {
            // The answer is taken before the page's sender goes.
            let _remote = remote;
            made.serve(cancel, signals).await;
        }));
        let (connected, connection) = watch::channel(None);
        let status = AbortOnDropHandle::new(tokio::spawn(async move {
            loop {
                let seen = watching.borrow_and_update().clone();
                match seen {
                    PeerStatus::Connecting => {}
                    PeerStatus::Connected => {
                        connected.send_replace(Some(true));
                    }
                    PeerStatus::Ended(_) => {
                        connected.send_replace(Some(false));
                        return;
                    }
                }
                if watching.changed().await.is_err() {
                    connected.send_replace(Some(false));
                    return;
                }
            }
        }));
        Ok(Self {
            local,
            connected: connection,
            _task: task,
            _status: status,
        })
    }

    /// Waits until the connection stands, or answers that it failed.
    pub async fn wait_connected(&mut self) -> bool {
        let state = self
            .connected
            .wait_for(|state| state.is_some())
            .await
            .expect("the page serves its peer");
        state.unwrap()
    }

    /// Waits until the connection fails or closes.
    pub async fn wait_closed(&mut self) {
        // A page whose task ended is closed too.
        let _ = self.connected.wait_for(|state| *state == Some(false)).await;
    }

    /// Opens a channel and sends `header` as its first message.
    pub async fn open(&self, header: serde_json::Value) -> PageChannel {
        let config = ChannelConfig {
            label: "operation".into(),
            ..Default::default()
        };
        let channel = self.channel(config).await;
        channel.text(&header.to_string());
        channel
    }

    /// Opens a live view's `video` channel, reliable and unordered, with its
    /// header.
    pub async fn video(&self) -> PageChannel {
        let config = ChannelConfig {
            label: "video".into(),
            ordered: false,
            ..Default::default()
        };
        let channel = self.channel(config).await;
        channel.text(r#"{"op":"video"}"#);
        channel
    }

    /// Opens a probe channel, unordered and never retransmitted, as the
    /// page's (`packages/web/src/direct/measure.ts`).
    pub async fn probe(&self) -> PageChannel {
        self.channel(probe_config()).await
    }

    async fn channel(&self, config: ChannelConfig) -> PageChannel {
        let (input, heard) = mpsc::unbounded_channel();
        let (sending, mut unbounded) = mpsc::unbounded_channel::<Output>();
        let (outputs, output) = mpsc::channel(OUTPUT_MESSAGES);
        // The test's sends are not async: they wait here for the channel's
        // room.
        tokio::spawn(async move {
            while let Some(message) = unbounded.recv().await {
                if outputs.send(message).await.is_err() {
                    return;
                }
            }
        });
        let (id, named) = tokio::sync::oneshot::channel();
        let open = LocalOpen {
            config,
            kind: Kind::Raw,
            first: None,
            input,
            output,
            cancel: CancellationToken::new(),
            credit: false,
            abort: None,
            id,
        };
        self.local.send(open).expect("the page serves its peer");
        named.await.expect("the channel opens");
        PageChannel {
            sending: Mutex::new(Some(sending)),
            heard,
        }
    }
}

impl PageChannel {
    fn send(&self, message: Output) {
        if let Some(sending) = &*self.sending.lock().expect("the channel's sender is whole") {
            // A channel that closed takes nothing more.
            let _ = sending.send(message);
        }
    }

    pub fn text(&self, text: &str) {
        self.send(Output::Text(text.to_owned()));
    }

    pub fn binary(&self, data: &[u8]) {
        self.send(Output::Binary(Bytes::copy_from_slice(data)));
    }

    /// Closes the channel once what was sent is out.
    pub fn close(&self) {
        self.sending.lock().expect("the channel's sender is whole").take();
    }

    /// The runner's next message, or its close.
    pub async fn next(&mut self) -> Heard {
        match self.heard.recv().await {
            Some(Input::Text(text)) => Heard::Text(text),
            Some(Input::Bytes(bytes)) => Heard::Binary(bytes),
            Some(Input::End | Input::Watch(_) | Input::Watched(_)) => unreachable!("a raw channel hears messages as they come"),
            None => Heard::Closed,
        }
    }

    /// The bytes the runner sends until it closes the channel.
    pub async fn bytes_to_end(&mut self) -> Vec<u8> {
        let mut bytes = Vec::new();
        loop {
            match self.next().await {
                Heard::Binary(chunk) => bytes.extend_from_slice(&chunk),
                Heard::Closed => return bytes,
                Heard::Text(text) => panic!("expected bytes, heard {text}"),
            }
        }
    }
}

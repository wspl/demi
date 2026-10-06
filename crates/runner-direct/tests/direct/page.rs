//! A page in process: a str0m offerer on `127.0.0.1` that connects to the
//! runner's peers as a browser does and speaks the channels' protocol.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use demi_runner_direct::{Direct, Refused};
use demi_runner_protocol::direct::Introduction;
use str0m::change::SdpAnswer;
use str0m::channel::{ChannelConfig, ChannelId};
use str0m::net::{Protocol, Receive};
use str0m::{Candidate, Event, IceConnectionState, Input, Output, Rtc, RtcConfig};
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::time::Instant;
use tokio_util::task::AbortOnDropHandle;

fn now() -> std::time::Instant {
    Instant::now().into_std()
}

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

enum Command {
    Open {
        heard: mpsc::UnboundedSender<Heard>,
        opened: oneshot::Sender<ChannelId>,
    },
    Send {
        id: ChannelId,
        binary: bool,
        data: Vec<u8>,
    },
    Close {
        id: ChannelId,
    },
}

/// The page's peer.
pub struct Page {
    commands: mpsc::UnboundedSender<Command>,
    /// Whether the connection stands: true once it connected, false once it
    /// failed or closed.
    pub connected: watch::Receiver<Option<bool>>,
    _task: AbortOnDropHandle<()>,
}

/// A channel of the page's.
pub struct PageChannel {
    id: ChannelId,
    commands: mpsc::UnboundedSender<Command>,
    heard: mpsc::UnboundedReceiver<Heard>,
}

impl Page {
    /// Offers a peer `peer` to `direct`, as the backend's introduction
    /// carries it, and serves the connection the answer makes.
    pub async fn offer(direct: &Direct, peer: &str, introduction: Introduction) -> Result<Self, Refused> {
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let local = socket.local_addr().unwrap();
        let mut rtc = RtcConfig::new()
            .set_crypto_provider(Arc::new(str0m::crypto::from_feature_flags()))
            .build(now());
        rtc.add_local_candidate(Candidate::host(local, "udp").unwrap());
        let mut changes = rtc.sdp_api();
        // A first channel, so that the offer carries one, as the page's.
        changes.add_channel("first".into());
        let (offer, pending) = changes.apply().unwrap();
        let answer = direct
            .offer(peer.into(), offer.to_sdp_string(), introduction)
            .await?;
        let answer = SdpAnswer::from_sdp_string(&answer).unwrap();
        rtc.sdp_api().accept_answer(pending, answer).unwrap();
        let (commands, received) = mpsc::unbounded_channel();
        let (connected, watching) = watch::channel(None);
        let task = AbortOnDropHandle::new(tokio::spawn(serve(rtc, socket, received, connected)));
        Ok(Self {
            commands,
            connected: watching,
            _task: task,
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
        let (heard, hearing) = mpsc::unbounded_channel();
        let (opened, open) = oneshot::channel();
        self.commands.send(Command::Open { heard, opened }).unwrap();
        let channel = PageChannel {
            id: open.await.expect("the channel opens"),
            commands: self.commands.clone(),
            heard: hearing,
        };
        channel.text(&header.to_string());
        channel
    }
}

impl PageChannel {
    pub fn text(&self, text: &str) {
        let _ = self.commands.send(Command::Send {
            id: self.id,
            binary: false,
            data: text.as_bytes().to_vec(),
        });
    }

    pub fn binary(&self, data: &[u8]) {
        let _ = self.commands.send(Command::Send {
            id: self.id,
            binary: true,
            data: data.to_vec(),
        });
    }

    pub fn close(&self) {
        let _ = self.commands.send(Command::Close { id: self.id });
    }

    /// The runner's next message, or its close.
    pub async fn next(&mut self) -> Heard {
        self.heard.recv().await.unwrap_or(Heard::Closed)
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

async fn serve(
    mut rtc: Rtc,
    socket: UdpSocket,
    mut commands: mpsc::UnboundedReceiver<Command>,
    connected: watch::Sender<Option<bool>>,
) {
    let local = socket.local_addr().unwrap();
    let mut channels: HashMap<ChannelId, mpsc::UnboundedSender<Heard>> = HashMap::new();
    let mut opening: HashMap<ChannelId, oneshot::Sender<ChannelId>> = HashMap::new();
    let mut unsent: Vec<(ChannelId, bool, Vec<u8>)> = Vec::new();
    let mut buffer = vec![0; 2048];
    loop {
        // What waits for room goes out first, in order.
        let waiting = std::mem::take(&mut unsent);
        for (id, binary, data) in waiting {
            let written = rtc
                .channel(id)
                .map(|mut channel| channel.write(binary, &data).unwrap_or(false))
                .unwrap_or(true);
            if !written {
                unsent.push((id, binary, data));
            }
        }
        let timeout = loop {
            if !rtc.is_alive() {
                connected.send_replace(Some(false));
                return;
            }
            match rtc.poll_output().unwrap() {
                Output::Timeout(timeout) => break timeout,
                Output::Transmit(transmit) => {
                    let _ = socket.send_to(&transmit.contents, transmit.destination).await;
                }
                Output::Event(event) => match event {
                    Event::Connected => {
                        connected.send_replace(Some(true));
                    }
                    Event::Closed | Event::IceConnectionStateChange(IceConnectionState::Disconnected) => {
                        connected.send_replace(Some(false));
                        return;
                    }
                    Event::ChannelOpen(id, _) => {
                        if let Some(opened) = opening.remove(&id) {
                            let _ = opened.send(id);
                        }
                    }
                    Event::ChannelData(data) => {
                        if let Some(heard) = channels.get(&data.id) {
                            let message = if data.binary {
                                Heard::Binary(Bytes::from(data.data))
                            } else {
                                Heard::Text(String::from_utf8(data.data).unwrap())
                            };
                            let _ = heard.send(message);
                        }
                    }
                    Event::ChannelClose(id) => {
                        if let Some(heard) = channels.remove(&id) {
                            let _ = heard.send(Heard::Closed);
                        }
                    }
                    _ => {}
                },
            }
        };
        let wait = Instant::from_std(timeout).max(Instant::now());
        let retry = if unsent.is_empty() {
            wait
        } else {
            wait.min(Instant::now() + Duration::from_millis(1))
        };
        tokio::select! {
            command = commands.recv() => {
                match command {
                    Some(Command::Open { heard, opened }) => {
                        let id = rtc.direct_api().create_data_channel(ChannelConfig {
                            label: "operation".into(),
                            ..Default::default()
                        });
                        channels.insert(id, heard);
                        opening.insert(id, opened);
                    }
                    Some(Command::Send { id, binary, data }) => unsent.push((id, binary, data)),
                    Some(Command::Close { id }) => rtc.direct_api().close_data_channel(id),
                    None => return,
                }
                // What the command queued goes out now, not at str0m's next
                // timer.
                if rtc.handle_input(Input::Timeout(now())).is_err() {
                    connected.send_replace(Some(false));
                    return;
                }
            }
            received = socket.recv_from(&mut buffer) => {
                let (length, source): (usize, SocketAddr) = received.unwrap();
                let Ok(contents) = buffer[..length].try_into() else {
                    continue;
                };
                let receive = Receive {
                    proto: Protocol::Udp,
                    source,
                    destination: local,
                    contents,
                };
                let handled = rtc
                    .handle_input(Input::Receive(now(), receive))
                    .and_then(|()| rtc.handle_input(Input::Timeout(now())));
                if handled.is_err() {
                    connected.send_replace(Some(false));
                    return;
                }
            }
            () = tokio::time::sleep_until(retry) => {
                if rtc.handle_input(Input::Timeout(now())).is_err() {
                    connected.send_replace(Some(false));
                    return;
                }
            }
        }
    }
}

//! The resolver a Cloud's DNS reaches: the firewall redirects every question
//! a Cloud asks on port 53, to whatever address, to this resolver on the
//! Cloud's slot's gateway (`super::ruleset`), which answers it by the network
//! rules the backend last named
//! (`crates/machine-manager-protocol/src/network.rs`):
//!
//! ```text
//! Cloud ── question ──> its gateway ──> a name the rules allow:
//!                                         forward it upstream, open
//!                                         each address of the answer,
//!                                         then answer
//!                                       another name, with the internet
//!                                         allowed: forward and answer
//!                                       another name: REFUSED
//! ```
//!
//! An address is opened before the Cloud gets the answer, so the connection
//! that follows the answer finds it open. It stays open for the answer's TTL,
//! and for at least [`FLOOR`], since programs keep an address after its TTL:
//! the JVM caches one for 30 seconds whatever the TTL says. Every new answer
//! renews it. An open connection outlives it (`ct state established`).
//!
//! A name the rules do not allow is refused when they leave out the
//! internet, so a Cloud cannot carry data out in the names it asks about. So
//! is a message of more than one question, which an upstream resolver might
//! answer for a name that was not allowed.
//!
//! The resolver keeps the addresses it opened, so a replaced table gets them
//! back with what remains of their time; a change of the rules closes them,
//! and the next answer for each allowed name opens it again.
//!
//! One lock orders every change of the firewall's sets: a table replaced
//! while an answer opens its addresses would otherwise drop them when the
//! addition lands first, with the record saying they are open. An answer
//! decides again under the lock when the rules changed while it was asked
//! upstream, so an answer the new rules refuse opens nothing and is refused.

use std::{
    cell::RefCell,
    collections::HashMap,
    fmt,
    net::{Ipv4Addr, SocketAddr},
    path::PathBuf,
    rc::{Rc, Weak},
    time::Duration,
};

use demi_machine_manager_protocol::NetworkRules;
use hickory_proto::{
    op::{Message, OpCode, ResponseCode},
    rr::RData,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream, UdpSocket},
    sync::{Mutex, MutexGuard},
    time::{Instant, timeout, timeout_at},
};
use tokio_util::task::AbortOnDropHandle;

use super::{
    ruleset::{self, Named, RESOLVER_PORT},
    slots::Slot,
};
use crate::blocking;

/// The shortest time an opened address stays open.
const FLOOR: Duration = Duration::from_secs(300);
/// The longest: an answer that says longer is asked again within a day.
const CEILING: Duration = Duration::from_secs(86_400);
/// How long one upstream resolver has to answer before the next is asked:
/// within the five seconds a Cloud's libc waits for an answer before it asks
/// again, so a second resolver still answers in time.
const UPSTREAM_WAIT: Duration = Duration::from_secs(2);
/// How long a Cloud's TCP connection to the resolver stays open between
/// questions, so a client that leaves it open does not hold a task forever.
const TCP_IDLE: Duration = Duration::from_secs(10);
/// The largest DNS message, over UDP or TCP.
const MESSAGE_BYTES: usize = 65_535;

/// The transport a question came over, which forwards it upstream.
#[derive(Clone, Copy)]
enum Transport {
    Udp,
    Tcp,
}

/// What the rules make of a name.
enum Decision {
    Refuse,
    /// Answer, opening nothing.
    Forward,
    /// Answer, opening the answer's addresses on these ports, or on every
    /// port.
    Open(Option<Vec<u16>>),
}

/// The Clouds' resolver: what the rules allow, the addresses it opened, and
/// a UDP and a TCP listener on each running Cloud's gateway.
pub struct Resolver {
    this: Weak<Resolver>,
    upstream: Vec<Ipv4Addr>,
    nft: PathBuf,
    rules: RefCell<NetworkRules>,
    /// Counts the changes of the rules, so an answer knows whether the rules
    /// it was decided by still hold.
    generation: RefCell<u64>,
    /// Each opened address, on every port or one, with when it closes.
    opened: RefCell<HashMap<(Ipv4Addr, Option<u16>), Instant>>,
    /// Held across every change of the firewall's table or sets.
    firewall: Mutex<()>,
    /// The listeners of each slot, by its index.
    listeners: RefCell<HashMap<u16, [AbortOnDropHandle<()>; 2]>>,
}

impl fmt::Debug for Resolver {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Resolver")
            .field("upstream", &self.upstream)
            .field("rules", &self.rules)
            .finish_non_exhaustive()
    }
}

impl Resolver {
    /// A resolver that forwards to `upstream` and opens addresses with
    /// `nft`, which allows the internet alone until the backend names the
    /// rules.
    pub fn new(upstream: Vec<Ipv4Addr>, nft: PathBuf) -> Rc<Self> {
        Rc::new_cyclic(|this| Self {
            this: this.clone(),
            upstream,
            nft,
            rules: RefCell::default(),
            generation: RefCell::default(),
            opened: RefCell::default(),
            firewall: Mutex::default(),
            listeners: RefCell::default(),
        })
    }

    /// Holds the firewall: the caller replaces the table while it holds it.
    pub async fn firewall(&self) -> MutexGuard<'_, ()> {
        self.firewall.lock().await
    }

    /// Takes `rules` once the table that applies them is installed; a change
    /// closes every address it opened, which that table left out.
    pub fn set_rules(&self, rules: &NetworkRules) {
        if *self.rules.borrow() != *rules {
            *self.rules.borrow_mut() = rules.clone();
            *self.generation.borrow_mut() += 1;
            self.opened.borrow_mut().clear();
        }
    }

    /// The addresses it opened that are still open, with the time left to
    /// each, which a table for `rules` keeps: none when `rules` are not the
    /// ones it follows.
    pub fn named(&self, rules: &NetworkRules) -> Vec<Named> {
        if *self.rules.borrow() != *rules {
            return Vec::new();
        }
        let now = Instant::now();
        self.opened
            .borrow()
            .iter()
            .filter_map(|((address, port), closes)| {
                let left = u32::try_from(closes.saturating_duration_since(now).as_secs()).ok()?;
                (left > 0).then_some(Named {
                    address: *address,
                    port: *port,
                    timeout: left,
                })
            })
            .collect()
    }

    /// Answers on `slot`'s gateway, once its address is up; a slot that has
    /// its listeners keeps them.
    pub async fn listen(&self, slot: &Slot) -> std::io::Result<()> {
        if self.listeners.borrow().contains_key(&slot.index) {
            return Ok(());
        }
        let bound = |error: std::io::Error| {
            std::io::Error::new(
                error.kind(),
                format!("The Clouds' resolver cannot listen on {}:{RESOLVER_PORT}: {error}", slot.gateway),
            )
        };
        let udp = Rc::new(UdpSocket::bind((slot.gateway, RESOLVER_PORT)).await.map_err(bound)?);
        let tcp = TcpListener::bind((slot.gateway, RESOLVER_PORT)).await.map_err(bound)?;
        let (this, that) = (self.this.clone(), self.this.clone());
        let handles = [
            AbortOnDropHandle::new(tokio::task::spawn_local(serve_udp(this, udp))),
            AbortOnDropHandle::new(tokio::task::spawn_local(serve_tcp(that, tcp))),
        ];
        self.listeners.borrow_mut().insert(slot.index, handles);
        Ok(())
    }

    /// Stops answering on `slot`'s gateway. Its sockets close at once: the
    /// questions still being answered hold them weakly, so the slot's next
    /// Cloud can listen there again.
    pub fn stop(&self, slot: &Slot) {
        self.listeners.borrow_mut().remove(&slot.index);
    }

    /// What the rules make of `name`, with the generation of the rules.
    fn decide(&self, name: &str) -> (u64, Decision) {
        let rules = self.rules.borrow();
        let decision = match rules.ports_of(name) {
            Some(ports) => Decision::Open(ports),
            None if rules.internet => Decision::Forward,
            None => Decision::Refuse,
        };
        (*self.generation.borrow(), decision)
    }

    /// The answer to `query`, which came over `transport`; none for one too
    /// short to carry an ID.
    async fn answer(&self, query: &[u8], transport: Transport) -> Option<Vec<u8>> {
        let message = match Message::from_vec(query) {
            Ok(message) if message.queries.len() == 1 => message,
            Ok(message) => return reply(&message, ResponseCode::FormErr),
            Err(_) => return format_error(query),
        };
        let name = message.queries[0].name().to_ascii();
        let (generation, decision) = self.decide(&name);
        if let Decision::Refuse = decision {
            return reply(&message, ResponseCode::Refused);
        }
        let Some(answer) = self.forward(query, transport).await else {
            return reply(&message, ResponseCode::ServFail);
        };
        let _firewall = self.firewall.lock().await;
        let decision = match *self.generation.borrow() == generation {
            true => decision,
            false => self.decide(&name).1,
        };
        match decision {
            Decision::Refuse => return reply(&message, ResponseCode::Refused),
            Decision::Forward => {}
            Decision::Open(ports) => {
                if let Ok(answered) = Message::from_vec(&answer) {
                    self.open(&answered, ports).await;
                }
            }
        }
        Some(answer)
    }

    /// Opens each address `answer` gives, on `ports` or on every port, for
    /// the record's TTL within [`FLOOR`] and [`CEILING`]; an address open for
    /// more than half that time already is left as it is. The caller holds
    /// the firewall.
    async fn open(&self, answer: &Message, ports: Option<Vec<u16>>) {
        let now = Instant::now();
        let ports: Vec<Option<u16>> = match ports {
            None => vec![None],
            Some(ports) => ports.into_iter().map(Some).collect(),
        };
        let mut named = Vec::new();
        {
            let mut opened = self.opened.borrow_mut();
            opened.retain(|_, closes| *closes > now);
            for record in &answer.answers {
                let RData::A(address) = &record.data else {
                    continue;
                };
                let open_for = Duration::from_secs(u64::from(record.ttl)).clamp(FLOOR, CEILING);
                for port in &ports {
                    let key = (address.0, *port);
                    if opened.get(&key).is_some_and(|closes| *closes >= now + open_for / 2) {
                        continue;
                    }
                    opened.insert(key, now + open_for);
                    named.push(Named {
                        address: address.0,
                        port: *port,
                        timeout: u32::try_from(open_for.as_secs()).expect("a day fits"),
                    });
                }
            }
        }
        if named.is_empty() {
            return;
        }
        let batch = ruleset::add_named(&named);
        let nft = self.nft.clone();
        if let Err(error) = blocking::run(move |_| super::linux::apply(&nft, &batch)).await {
            tracing::warn!(event = "cloud_resolver.open_failed", %error, "the addresses of an allowed name were not opened");
        }
    }

    /// The upstream resolvers' answer to `query`, asked in turn; none when
    /// none answers.
    async fn forward(&self, query: &[u8], transport: Transport) -> Option<Vec<u8>> {
        for server in &self.upstream {
            let server = SocketAddr::from((*server, 53));
            let answer = match transport {
                Transport::Udp => forward_udp(server, query).await,
                Transport::Tcp => timeout(UPSTREAM_WAIT, forward_tcp(server, query)).await.ok().flatten(),
            };
            if answer.is_some() {
                return answer;
            }
        }
        None
    }
}

/// An answer to `message` with `code` and its questions.
fn reply(message: &Message, code: ResponseCode) -> Option<Vec<u8>> {
    let mut reply = Message::error_msg(message.metadata.id, message.metadata.op_code, code);
    reply.metadata.recursion_desired = message.metadata.recursion_desired;
    reply.queries = message.queries.clone();
    reply.to_vec().ok()
}

/// A format error for `query`, which is no DNS message, so the client fails
/// at once rather than waiting; none for one too short to carry an ID.
fn format_error(query: &[u8]) -> Option<Vec<u8>> {
    let id = u16::from_be_bytes(query.get(..2)?.try_into().ok()?);
    Message::error_msg(id, OpCode::Query, ResponseCode::FormErr).to_vec().ok()
}

/// `server`'s answer to `query` over UDP: the first datagram from it with the
/// query's ID, within [`UPSTREAM_WAIT`].
async fn forward_udp(server: SocketAddr, query: &[u8]) -> Option<Vec<u8>> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).await.ok()?;
    socket.connect(server).await.ok()?;
    socket.send(query).await.ok()?;
    let deadline = Instant::now() + UPSTREAM_WAIT;
    let mut buffer = vec![0; MESSAGE_BYTES];
    loop {
        let length = timeout_at(deadline, socket.recv(&mut buffer)).await.ok()?.ok()?;
        if length >= 2 && buffer[..2] == query[..2] {
            buffer.truncate(length);
            return Some(buffer);
        }
    }
}

/// `server`'s answer to `query` over TCP.
async fn forward_tcp(server: SocketAddr, query: &[u8]) -> Option<Vec<u8>> {
    let mut stream = TcpStream::connect(server).await.ok()?;
    write_framed(&mut stream, query).await.ok()?;
    read_framed(&mut stream).await.ok()
}

/// Answers each question that reaches `socket`, each in a task of its own.
async fn serve_udp(resolver: Weak<Resolver>, socket: Rc<UdpSocket>) {
    let mut buffer = vec![0; MESSAGE_BYTES];
    loop {
        let (length, peer) = match socket.recv_from(&mut buffer).await {
            Ok(received) => received,
            Err(error) => {
                tracing::warn!(event = "cloud_resolver.receive_failed", %error, "a question to the Clouds' resolver was lost");
                continue;
            }
        };
        let query = buffer[..length].to_vec();
        let (resolver, socket) = (resolver.clone(), Rc::downgrade(&socket));
        tokio::task::spawn_local(async move {
            let Some(resolver) = resolver.upgrade() else {
                return;
            };
            let answer = resolver.answer(&query, Transport::Udp).await;
            if let (Some(answer), Some(socket)) = (answer, socket.upgrade()) {
                let _ = socket.send_to(&answer, peer).await;
            }
        });
    }
}

/// Answers the questions of each connection to `listener`, each connection
/// in a task of its own.
async fn serve_tcp(resolver: Weak<Resolver>, listener: TcpListener) {
    loop {
        let mut stream = match listener.accept().await {
            Ok((stream, _)) => stream,
            Err(error) => {
                tracing::warn!(event = "cloud_resolver.accept_failed", %error, "a connection to the Clouds' resolver was lost");
                continue;
            }
        };
        let resolver = resolver.clone();
        tokio::task::spawn_local(async move {
            while let Ok(Ok(query)) = timeout(TCP_IDLE, read_framed(&mut stream)).await {
                let Some(resolver) = resolver.upgrade() else {
                    return;
                };
                let Some(answer) = resolver.answer(&query, Transport::Tcp).await else {
                    return;
                };
                if write_framed(&mut stream, &answer).await.is_err() {
                    return;
                }
            }
        });
    }
}

/// A DNS message over TCP: its length in two bytes, then the message.
async fn read_framed(stream: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    let length = stream.read_u16().await?;
    let mut message = vec![0; usize::from(length)];
    stream.read_exact(&mut message).await?;
    Ok(message)
}

async fn write_framed(stream: &mut TcpStream, message: &[u8]) -> std::io::Result<()> {
    let length = u16::try_from(message.len()).map_err(std::io::Error::other)?;
    let mut framed = Vec::with_capacity(message.len() + 2);
    framed.extend_from_slice(&length.to_be_bytes());
    framed.extend_from_slice(message);
    stream.write_all(&framed).await
}

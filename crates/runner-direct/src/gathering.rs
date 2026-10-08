//! A peer's server-reflexive candidates (`direct-channel.md` § Making the
//! channel): the runner asks each STUN server the backend names, from each
//! of the peer's sockets that can reach it, for the address the internet
//! sees that socket at, and offers each new one to the page. A socket on
//! `127.0.0.1` asks only a server on loopback, as a test's is, and a socket
//! on the local network only a server off it. A request goes again after
//! 0.5, 1 and 2 seconds without an answer, then the socket gives up on that
//! server; the peer goes on with what it has.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use demi_runner_protocol::direct::StunUrl;
use tokio::net::UdpSocket;
use tokio::time::Instant;

/// The waits before each resend of an unanswered request; after the last,
/// the request is given up.
const RESENDS: [Duration; 3] = [
    Duration::from_millis(500),
    Duration::from_secs(1),
    Duration::from_secs(2),
];

/// A STUN transaction's id (RFC 8489 § 5).
type TransId = [u8; 12];

/// The magic cookie every STUN message carries (RFC 8489 § 5).
const MAGIC_COOKIE: u32 = 0x2112_A442;
/// A binding request's and a binding success response's message types.
const BINDING_REQUEST: u16 = 0x0001;
const BINDING_SUCCESS: u16 = 0x0101;
/// The attributes that name the address the server saw.
const MAPPED_ADDRESS: u16 = 0x0001;
const XOR_MAPPED_ADDRESS: u16 = 0x0020;

// str0m's STUN messages are ICE's: its parser refuses a binding response
// without MESSAGE-INTEGRITY, which a STUN server's answer to a plain request
// never has, so gathering speaks the two messages it needs itself.

/// A binding request: the header alone, with no attribute.
fn binding_request(id: &TransId) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(20);
    bytes.extend_from_slice(&BINDING_REQUEST.to_be_bytes());
    bytes.extend_from_slice(&0u16.to_be_bytes());
    bytes.extend_from_slice(&MAGIC_COOKIE.to_be_bytes());
    bytes.extend_from_slice(id);
    bytes
}

/// The id and the IPv4 address a binding success response names, its
/// XOR-MAPPED-ADDRESS, or else its MAPPED-ADDRESS; none for anything else.
fn binding_success(contents: &[u8]) -> Option<(TransId, SocketAddr)> {
    let header = contents.get(..20)?;
    let typ = u16::from_be_bytes([header[0], header[1]]);
    let length = usize::from(u16::from_be_bytes([header[2], header[3]]));
    let cookie = u32::from_be_bytes([header[4], header[5], header[6], header[7]]);
    if typ != BINDING_SUCCESS || cookie != MAGIC_COOKIE {
        return None;
    }
    let id: TransId = header[8..20].try_into().ok()?;
    let mut attributes = contents.get(20..20 + length)?;
    let mut mapped = None;
    while attributes.len() >= 4 {
        let kind = u16::from_be_bytes([attributes[0], attributes[1]]);
        let size = usize::from(u16::from_be_bytes([attributes[2], attributes[3]]));
        let value = attributes.get(4..4 + size)?;
        match kind {
            XOR_MAPPED_ADDRESS => return ipv4(value, true).map(|address| (id, address)),
            MAPPED_ADDRESS => mapped = ipv4(value, false),
            _ => {}
        }
        // Each attribute is padded to four bytes.
        let next = (4 + size).next_multiple_of(4);
        attributes = attributes.get(next..).unwrap_or_default();
    }
    mapped.map(|address| (id, address))
}

/// An address attribute's IPv4 address, XORed with the magic cookie when
/// `xor`; none for another family.
fn ipv4(value: &[u8], xor: bool) -> Option<SocketAddr> {
    let value = value.get(..8)?;
    // Family 0x01 is IPv4.
    if value[1] != 0x01 {
        return None;
    }
    let mut port = u16::from_be_bytes([value[2], value[3]]);
    let mut ip = u32::from_be_bytes([value[4], value[5], value[6], value[7]]);
    if xor {
        port ^= (MAGIC_COOKIE >> 16) as u16;
        ip ^= MAGIC_COOKIE;
    }
    Some(SocketAddr::from((std::net::Ipv4Addr::from(ip), port)))
}

/// One socket's request to one server, until it is answered or given up.
struct Request {
    socket: Arc<UdpSocket>,
    server: SocketAddr,
    bytes: Vec<u8>,
    /// How many resends went.
    resent: usize,
    next: Instant,
}

/// The requests a peer waits on.
#[derive(Default)]
pub(crate) struct Gathering {
    pending: HashMap<TransId, Request>,
}

/// The servers' IPv4 addresses, each server's first; a server whose host
/// does not resolve is left out.
pub(crate) async fn resolve(servers: Vec<StunUrl>) -> Vec<SocketAddr> {
    let mut addresses = Vec::new();
    for server in servers {
        match tokio::net::lookup_host((server.host(), server.port())).await {
            Ok(mut found) => {
                if let Some(address) = found.find(SocketAddr::is_ipv4) {
                    addresses.push(address);
                }
            }
            // A server out of reach gives no address; the others may.
            Err(error) => tracing::debug!("STUN server {server} does not resolve: {error}"),
        }
    }
    addresses
}

impl Gathering {
    /// Sends a request from each of `sockets` to each of `servers` it can
    /// reach.
    pub(crate) async fn start(&mut self, servers: &[SocketAddr], sockets: &[Arc<UdpSocket>]) {
        let now = Instant::now();
        for socket in sockets {
            let Ok(base) = socket.local_addr() else {
                continue;
            };
            for server in servers {
                if base.ip().is_loopback() != server.ip().is_loopback() {
                    continue;
                }
                let mut id = TransId::default();
                id.copy_from_slice(&uuid::Uuid::new_v4().as_bytes()[..12]);
                let request = Request {
                    socket: socket.clone(),
                    server: *server,
                    bytes: binding_request(&id),
                    resent: 0,
                    next: now + RESENDS[0],
                };
                send(&request).await;
                self.pending.insert(id, request);
            }
        }
    }

    /// When the next resend is due; none while nothing waits.
    pub(crate) fn next(&self) -> Option<Instant> {
        self.pending.values().map(|request| request.next).min()
    }

    /// Sends again each request whose wait passed, and gives up those that
    /// waited the last time.
    pub(crate) async fn resend(&mut self) {
        let now = Instant::now();
        let mut given_up = Vec::new();
        for (id, request) in &mut self.pending {
            if request.next > now {
                continue;
            }
            request.resent += 1;
            match RESENDS.get(request.resent) {
                Some(wait) => {
                    request.next = now + *wait;
                    send(request).await;
                }
                None => given_up.push(*id),
            }
        }
        for id in given_up {
            if let Some(request) = self.pending.remove(&id) {
                tracing::debug!("STUN server {} did not answer", request.server);
            }
        }
    }

    /// The address a server saw the socket at `destination` at, when
    /// `contents` is a server's answer to one of the requests; the request
    /// is done.
    pub(crate) fn answer(&mut self, destination: SocketAddr, contents: &[u8]) -> Option<SocketAddr> {
        let (id, mapped) = binding_success(contents)?;
        let request = self.pending.get(&id)?;
        if request.socket.local_addr().ok() != Some(destination) {
            return None;
        }
        self.pending.remove(&id);
        Some(mapped)
    }
}

async fn send(request: &Request) {
    if let Err(error) = request.socket.send_to(&request.bytes, request.server).await {
        // A route that is down now may be up at the resend.
        tracing::debug!("a STUN request to {} could not go: {error}", request.server);
    }
}

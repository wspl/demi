//! The addresses a runner offers a page (`direct-channel.md` § Making the
//! channel): `127.0.0.1`, and the IPv4 addresses of its interfaces that are
//! up, can broadcast and are not point-to-point. A VPN's tunnel interface is
//! point-to-point, so the fake address a VPN client gives is never offered:
//! WebKit's encryption handshake times out over one.

use std::net::Ipv4Addr;

use if_addrs::{IfAddr, Interface};

/// Where a runner's peers bind their sockets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Addresses {
    /// `127.0.0.1` and the qualifying addresses of the interfaces as they
    /// are when a peer is made.
    Interfaces,
    /// These addresses only, as a test binds.
    Only(Vec<Ipv4Addr>),
}

impl Addresses {
    /// The addresses to bind a new peer's sockets on.
    pub(crate) fn current(&self) -> Vec<Ipv4Addr> {
        match self {
            Self::Only(addresses) => addresses.clone(),
            Self::Interfaces => {
                let interfaces = match if_addrs::get_if_addrs() {
                    Ok(interfaces) => interfaces,
                    Err(error) => {
                        // Loopback alone still serves a browser on this
                        // machine.
                        tracing::warn!("the network interfaces could not be listed: {error}");
                        Vec::new()
                    }
                };
                offered(&interfaces)
            }
        }
    }
}

/// `127.0.0.1`, then each IPv4 address of `interfaces` that is up, can
/// broadcast and is not point-to-point, each once.
pub fn offered(interfaces: &[Interface]) -> Vec<Ipv4Addr> {
    let mut addresses = vec![Ipv4Addr::LOCALHOST];
    for interface in interfaces {
        let IfAddr::V4(address) = &interface.addr else {
            continue;
        };
        let qualifies = interface.is_oper_up()
            && address.broadcast.is_some()
            && !interface.is_p2p()
            && !address.ip.is_loopback();
        if qualifies && !addresses.contains(&address.ip) {
            addresses.push(address.ip);
        }
    }
    addresses
}

//! Which private destinations Clouds reach: the setting the master chooses,
//! which the backend stores and the manager's firewall applies
//! (`crates/machine-manager/src/network/ruleset.rs`).
//!
//! A destination is written as the master types it, an IPv4 address or
//! network with an optional `:port`, such as `10.20.0.7:22` or
//! `10.20.0.0/16`, and parsed here once for both ends. Names are not
//! destinations: the firewall checks the address a connection goes to, so a
//! public name that resolves to a private address opens nothing the setting
//! did not.

use std::{fmt, net::Ipv4Addr, str::FromStr};

use ipnet::Ipv4Net;
use serde::{Deserialize, Serialize};

/// Which private destinations Clouds reach; the Claude Code machine reaches
/// none of them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "access", content = "destinations", rename_all = "snake_case")]
pub enum PrivateNetwork {
    /// None: Clouds reach only public destinations.
    #[default]
    Off,
    /// Every network of [`PRIVATE_NETWORKS`].
    Entire,
    /// Only these.
    Only(Vec<Destination>),
}

/// The private networks Entire Private Network opens: the RFC 1918 ranges and
/// the shared range of RFC 6598, where Tailscale's addresses are.
pub const PRIVATE_NETWORKS: [(Ipv4Addr, u8); 4] = [
    (Ipv4Addr::new(10, 0, 0, 0), 8),
    (Ipv4Addr::new(172, 16, 0, 0), 12),
    (Ipv4Addr::new(192, 168, 0, 0), 16),
    (Ipv4Addr::new(100, 64, 0, 0), 10),
];

/// The ranges no setting opens, since they reach the server itself or
/// nothing: "this network", loopback, link-local, where cloud metadata
/// services answer, multicast and the reserved range. The server's own
/// addresses and the sandbox pool, which is the other Clouds, are closed too;
/// the manager knows them.
pub const NEVER_REACHABLE: [(Ipv4Addr, u8); 5] = [
    (Ipv4Addr::new(0, 0, 0, 0), 8),
    (Ipv4Addr::new(127, 0, 0, 0), 8),
    (Ipv4Addr::new(169, 254, 0, 0), 16),
    (Ipv4Addr::new(224, 0, 0, 0), 4),
    (Ipv4Addr::new(240, 0, 0, 0), 4),
];

/// An address or network a Cloud may reach, on one TCP or UDP port or on
/// every port and protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Destination {
    pub network: Ipv4Net,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
}

/// Why a written destination is not one.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DestinationError {
    #[error("“{0}” is not an IPv4 address or network, such as 10.20.0.7 or 10.20.0.0/16")]
    Address(String),
    #[error("“{0}” is not a port from 1 to 65535")]
    Port(String),
    #[error("{0} is never reachable from a Cloud")]
    NeverReachable(String),
}

impl FromStr for Destination {
    type Err = DestinationError;

    /// Reads `10.20.0.7`, `10.20.0.0/16` or either with `:port`. A network
    /// written with host bits, `10.20.0.7/16`, is taken as the network it is
    /// in.
    fn from_str(written: &str) -> Result<Self, Self::Err> {
        let written = written.trim();
        let (address, port) = match written.rsplit_once(':') {
            Some((address, port)) => {
                let port = port
                    .parse::<u16>()
                    .ok()
                    .filter(|port| *port != 0)
                    .ok_or_else(|| DestinationError::Port(port.to_owned()))?;
                (address, Some(port))
            }
            None => (written, None),
        };
        let network = match address.parse::<Ipv4Net>() {
            Ok(network) => network.trunc(),
            Err(_) => address
                .parse::<Ipv4Addr>()
                .map(Ipv4Net::from)
                .map_err(|_| DestinationError::Address(address.to_owned()))?,
        };
        let destination = Self { network, port };
        if destination.never_reachable() {
            return Err(DestinationError::NeverReachable(destination.to_string()));
        }
        Ok(destination)
    }
}

impl Destination {
    /// Whether the destination touches a range of [`NEVER_REACHABLE`].
    pub fn never_reachable(&self) -> bool {
        NEVER_REACHABLE.iter().any(|(address, length)| {
            let range = Ipv4Net::new(*address, *length).expect("a valid prefix length");
            range.contains(&self.network.network()) || self.network.contains(&range.network())
        })
    }
}

impl fmt::Display for Destination {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.network.prefix_len() == 32 {
            write!(formatter, "{}", self.network.addr())?;
        } else {
            write!(formatter, "{}", self.network)?;
        }
        match self.port {
            Some(port) => write!(formatter, ":{port}"),
            None => Ok(()),
        }
    }
}

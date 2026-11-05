//! What Clouds reach: the rules the server's network policy gives the
//! manager, which its firewall (`crates/machine-manager/src/network/ruleset.rs`)
//! and its resolver (`crates/machine-manager/src/network/resolver.rs`) apply.
//!
//! An entry of a custom policy is written as the master types it, and parsed
//! here once for both ends:
//!
//! - an IPv4 address or network, such as `10.20.0.7` or `10.20.0.0/16`, which
//!   the firewall allows by address;
//! - a domain, such as `git.example.com`, or every name under one, such as
//!   `*.example.com`, which the resolver allows: it opens the addresses of
//!   each answer for an allowed name before the Cloud gets the answer.
//!
//! Either takes an optional `:port`, which allows that TCP or UDP port only.

use std::{fmt, net::Ipv4Addr, str::FromStr};

use ipnet::Ipv4Net;
use serde::{Deserialize, Serialize};

/// What Clouds reach; the Claude Code machine reaches the internet whatever
/// they are.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NetworkRules {
    /// Every public address.
    pub internet: bool,
    /// Every network of [`PRIVATE_NETWORKS`].
    pub private_network: bool,
    /// Every name of [`DEVELOPER_SERVICES`].
    pub developer_services: bool,
    pub allowed: Vec<Allowed>,
}

impl Default for NetworkRules {
    /// The internet alone, which Clouds reach until the backend names the
    /// rules.
    fn default() -> Self {
        Self {
            internet: true,
            private_network: false,
            developer_services: false,
            allowed: Vec::new(),
        }
    }
}

impl NetworkRules {
    /// The ports `name` is allowed on: `Some(None)` on every port, `Some`
    /// of the ports its entries name otherwise, `None` when no entry and no
    /// Developer Services' name covers it. `name` is compared without case
    /// and without its final dot.
    pub fn ports_of(&self, name: &str) -> Option<Option<Vec<u16>>> {
        let name = name.trim_end_matches('.').to_ascii_lowercase();
        if self.developer_services && DEVELOPER_SERVICES.iter().any(|service| name == *service || is_under(&name, service)) {
            return Some(None);
        }
        let mut ports = Vec::new();
        for allowed in &self.allowed {
            let Allowed::Domain(domain) = allowed else {
                continue;
            };
            if !domain.name.covers(&name) {
                continue;
            }
            match domain.port {
                None => return Some(None),
                Some(port) => ports.push(port),
            }
        }
        (!ports.is_empty()).then_some(Some(ports))
    }
}

/// The private networks Private Network opens: the RFC 1918 ranges and the
/// shared range of RFC 6598, where Tailscale's addresses are.
pub const PRIVATE_NETWORKS: [(Ipv4Addr, u8); 4] = [
    (Ipv4Addr::new(10, 0, 0, 0), 8),
    (Ipv4Addr::new(172, 16, 0, 0), 12),
    (Ipv4Addr::new(192, 168, 0, 0), 16),
    (Ipv4Addr::new(100, 64, 0, 0), 10),
];

/// The ranges no rule opens, since they reach the server itself or
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

/// Developer Services: Git hosts, package registries and the downloads of
/// build tools, each with every name under it, since a registry serves from
/// its subdomains. It starts from the list Codex publishes for the same
/// purpose (developers.openai.com/codex/cloud/agent-internet), and a release
/// may change it. It is what builds need, not a guard against data leaving:
/// GitHub accepts pushes. Settings › Cloud names the first three and counts
/// the rest.
pub const DEVELOPER_SERVICES: &[&str] = &[
    "github.com",
    "npmjs.org",
    "pypi.org",
    "alpinelinux.org",
    "anaconda.com",
    "apache.org",
    "apt.llvm.org",
    "archlinux.org",
    "azure.com",
    "bitbucket.org",
    "bower.io",
    "centos.org",
    "cocoapods.org",
    "continuum.io",
    "cpan.org",
    "crates.io",
    "debian.org",
    "docker.com",
    "docker.io",
    "dot.net",
    "dotnet.microsoft.com",
    "eclipse.org",
    "fedoraproject.org",
    "gcr.io",
    "ghcr.io",
    "githubusercontent.com",
    "gitlab.com",
    "golang.org",
    "google.com",
    "goproxy.io",
    "gradle.org",
    "hashicorp.com",
    "haskell.org",
    "hex.pm",
    "java.com",
    "java.net",
    "jcenter.bintray.com",
    "json-schema.org",
    "json.schemastore.org",
    "k8s.io",
    "launchpad.net",
    "maven.org",
    "mcr.microsoft.com",
    "metacpan.org",
    "microsoft.com",
    "nodejs.org",
    "npmjs.com",
    "nuget.org",
    "oracle.com",
    "packagecloud.io",
    "packages.microsoft.com",
    "packagist.org",
    "pkg.go.dev",
    "ppa.launchpad.net",
    "pub.dev",
    "pypa.io",
    "pypi.python.org",
    "pythonhosted.org",
    "quay.io",
    "ruby-lang.org",
    "rubyforge.org",
    "rubygems.org",
    "rubyonrails.org",
    "rustup.rs",
    "rvm.io",
    "sourceforge.net",
    "spring.io",
    "swift.org",
    "ubuntu.com",
    "visualstudio.com",
    "yarnpkg.com",
];

/// An entry of a custom policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum Allowed {
    Network(Destination),
    Domain(Domain),
}

/// An address or network a Cloud may reach, on one TCP or UDP port or on
/// every port and protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Destination {
    pub network: Ipv4Net,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
}

/// A domain whose addresses a Cloud may reach, on one TCP or UDP port or on
/// every port and protocol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Domain {
    pub name: DomainName,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
}

/// Which names a domain entry covers, in lowercase without a final dot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum DomainName {
    /// `git.example.com`: that name alone.
    Exact(String),
    /// `*.example.com`: every name under `example.com`, not the name itself.
    Under(String),
}

impl DomainName {
    /// Whether the entry covers `name`, which is lowercase without a final
    /// dot.
    pub fn covers(&self, name: &str) -> bool {
        match self {
            Self::Exact(exact) => name == exact,
            Self::Under(parent) => is_under(name, parent),
        }
    }
}

/// Whether `name` is a name under `parent`, such as `api.example.com` under
/// `example.com`.
fn is_under(name: &str, parent: &str) -> bool {
    name.strip_suffix(parent).is_some_and(|rest| rest.ends_with('.'))
}

/// Why a written entry is not one.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AllowedError {
    #[error("“{0}” is neither a domain, such as git.example.com or *.example.com, nor an IPv4 address or network, such as 10.20.0.0/16")]
    Entry(String),
    #[error("“{0}” is not a port from 1 to 65535")]
    Port(String),
    #[error("{0} is never reachable from a Cloud")]
    NeverReachable(String),
}

impl FromStr for Allowed {
    type Err = AllowedError;

    /// Reads an address, a network or a domain, each with an optional
    /// `:port`. A network written with host bits, `10.20.0.7/16`, is taken
    /// as the network it is in.
    fn from_str(written: &str) -> Result<Self, Self::Err> {
        let written = written.trim();
        let (target, port) = match written.rsplit_once(':') {
            Some((target, port)) => {
                let port = port
                    .parse::<u16>()
                    .ok()
                    .filter(|port| *port != 0)
                    .ok_or_else(|| AllowedError::Port(port.to_owned()))?;
                (target, Some(port))
            }
            None => (written, None),
        };
        if let Ok(network) = target.parse::<Ipv4Net>() {
            return Destination::checked(network.trunc(), port).map(Self::Network);
        }
        if let Ok(address) = target.parse::<Ipv4Addr>() {
            return Destination::checked(Ipv4Net::from(address), port).map(Self::Network);
        }
        let name = target.to_ascii_lowercase();
        let name = name.strip_suffix('.').unwrap_or(&name);
        let name = match name.strip_prefix("*.") {
            Some(parent) => DomainName::Under(parent.to_owned()),
            None => DomainName::Exact(name.to_owned()),
        };
        let (DomainName::Exact(text) | DomainName::Under(text)) = &name;
        if !is_host_name(text) {
            return Err(AllowedError::Entry(target.to_owned()));
        }
        Ok(Self::Domain(Domain { name, port }))
    }
}

/// Whether `name` is a host name: labels of letters, digits and inner
/// hyphens, 63 characters at most each and 253 in all, the last of which is
/// not all digits, so an address is never taken for a name.
fn is_host_name(name: &str) -> bool {
    let label = |label: &str| {
        (1..=63).contains(&label.len())
            && label.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            && !label.starts_with('-')
            && !label.ends_with('-')
    };
    name.len() <= 253
        && name.split('.').all(label)
        && name.rsplit('.').next().is_some_and(|last| !last.bytes().all(|byte| byte.is_ascii_digit()))
}

impl Destination {
    /// The destination, unless it touches a range of [`NEVER_REACHABLE`].
    fn checked(network: Ipv4Net, port: Option<u16>) -> Result<Self, AllowedError> {
        let destination = Self { network, port };
        if destination.never_reachable() {
            return Err(AllowedError::NeverReachable(destination.to_string()));
        }
        Ok(destination)
    }

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

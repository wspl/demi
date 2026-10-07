//! What the tests need of the engine (`testing.md` § Resources and
//! placement): fixture sites on loopback under names of their own, each
//! standing for a network, and the authority their certificates come from.

use std::collections::HashMap;

pub use crate::upstream::Space;

/// The names the tests' upstream requests reach: every one answers on
/// `127.0.0.1`, and counts as the network it is given, so a fixture can
/// stand for a public site.
#[derive(Debug, Default)]
pub struct Network {
    hosts: HashMap<String, Space>,
    authority: Vec<u8>,
}

impl Network {
    /// A network whose HTTPS fixtures present certificates `authority` (PEM)
    /// issued.
    pub fn new(authority: &[u8]) -> Self {
        Self {
            hosts: HashMap::new(),
            authority: authority.to_vec(),
        }
    }

    /// `name` resolves to loopback and counts as `space`.
    pub fn host(mut self, name: &str, space: Space) -> Self {
        self.hosts.insert(name.to_owned(), space);
        self
    }

    pub(crate) fn space(&self, host: &str) -> Option<Space> {
        self.hosts.get(host).copied()
    }

    pub(crate) fn authority(&self) -> &[u8] {
        &self.authority
    }
}

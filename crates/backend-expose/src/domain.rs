//! The domain expose hostnames live under (`expose.md` § Deployment).

use std::str::FromStr;

/// The domain, such as `expose.demi.example`: a DNS name, in lowercase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExposeDomain(String);

/// Why a text is not an expose domain.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("must be a domain name, such as expose.demi.example")]
pub struct NotExposeDomain;

impl FromStr for ExposeDomain {
    type Err = NotExposeDomain;

    fn from_str(text: &str) -> Result<Self, NotExposeDomain> {
        match url::Host::parse(text) {
            Ok(url::Host::Domain(domain)) if domain.split('.').all(|label| !label.is_empty()) => {
                Ok(Self(domain))
            }
            _ => Err(NotExposeDomain),
        }
    }
}

impl ExposeDomain {
    /// The domain, in lowercase.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The label an expose hostname puts before this domain: `host`, a
    /// `Host` header's host without its port, is `<label>.<domain>` in any
    /// case, and the label has no dot. `None` for any other host.
    pub fn label(&self, host: &str) -> Option<String> {
        let host = host.to_ascii_lowercase();
        let label = host.strip_suffix(self.0.as_str())?.strip_suffix('.')?;
        (!label.is_empty() && !label.contains('.')).then(|| label.to_owned())
    }
}

//! Sealed credentials (`crates/backend/src/config/secret.rs`): a typed JSON document encrypted with
//! AES-256-GCM under a key derived from the instance secret, bound to the row it belongs to.
//!
//! It is stored as one BLOB of a random 12-byte nonce, the ciphertext and the 16-byte tag.
//!
//! The plugins' secrets are sealed here too (`crates/backend-plugins/src/secrets.rs`), through the
//! product's side of the plugin host's port, and so are the values of the users' credentials.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use demi_web_api_protocol::ids::{CredentialId, ProviderId, UserId};

const NONCE_BYTES: usize = 12;

/// The key credentials are sealed under.
#[derive(Clone)]
pub struct VaultKey(Aes256Gcm);

/// The row a sealed value belongs to: its additional authenticated data, so
/// that a value copied into another row does not open.
#[derive(Debug, Clone, Copy)]
pub enum Row<'a> {
    /// An API-key entry's configuration.
    Config(&'a ProviderId),
    /// A subscription account's secret document.
    Secret(&'a ProviderId, &'a CredentialId),
    /// A plugin's secret for a user, by the plugin's id and the secret's
    /// name.
    PluginSecret {
        plugin: &'a str,
        user: &'a UserId,
        name: &'a str,
    },
    /// A user's credential's values, by the user and the credential's name.
    Credential { user: &'a UserId, name: &'a str },
    /// An automation's URL secret, by the automation's id.
    AutomationSecret(&'a str),
    /// The Stripe signing secret the user pasted into an automation, by the
    /// automation's id.
    AutomationStripeSecret(&'a str),
}

impl Row<'_> {
    /// The row's name as bytes: a label, then each id preceded by its length,
    /// so that no two rows share a name.
    fn name(&self) -> Vec<u8> {
        let (label, ids): (&[u8], Vec<&str>) = match self {
            Self::Config(provider) => (b"demi provider config", vec![provider.as_str()]),
            Self::Secret(provider, credential) => (
                b"demi account secret",
                vec![provider.as_str(), credential.as_str()],
            ),
            Self::PluginSecret { plugin, user, name } => {
                (b"demi plugin secret", vec![*plugin, user.as_str(), *name])
            }
            Self::Credential { user, name } => (b"demi credential", vec![user.as_str(), *name]),
            Self::AutomationSecret(automation) => (b"demi automation secret", vec![*automation]),
            Self::AutomationStripeSecret(automation) => {
                (b"demi automation stripe secret", vec![*automation])
            }
        };
        let mut name = label.to_vec();
        for id in ids {
            let length = u32::try_from(id.len()).expect("an id is shorter than 4 GiB");
            name.extend_from_slice(&length.to_be_bytes());
            name.extend_from_slice(id.as_bytes());
        }
        name
    }
}

/// A sealed value that does not open: it was altered, moved to another row,
/// or sealed under another instance secret. Nothing replaces it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the sealed value does not open")]
pub struct Unsealable;

impl VaultKey {
    pub fn new(key: [u8; 32]) -> Self {
        Self(Aes256Gcm::new(&key.into()))
    }

    /// `plaintext` sealed for `row`.
    pub fn seal(&self, row: Row<'_>, plaintext: &[u8]) -> Vec<u8> {
        let nonce: [u8; NONCE_BYTES] = rand::random();
        let aad = row.name();
        // AES-GCM refuses only messages beyond 64 GiB.
        let ciphertext = self
            .0
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: plaintext,
                    aad: &aad,
                },
            )
            .expect("a credential document is small enough to seal");
        let mut sealed = nonce.to_vec();
        sealed.extend_from_slice(&ciphertext);
        sealed
    }

    /// The plaintext `sealed` holds for `row`.
    pub fn open(&self, row: Row<'_>, sealed: &[u8]) -> Result<Vec<u8>, Unsealable> {
        if sealed.len() < NONCE_BYTES {
            return Err(Unsealable);
        }
        let (nonce, ciphertext) = sealed.split_at(NONCE_BYTES);
        let aad = row.name();
        self.0
            .decrypt(
                Nonce::from_slice(nonce),
                Payload {
                    msg: ciphertext,
                    aad: &aad,
                },
            )
            .map_err(|_| Unsealable)
    }
}

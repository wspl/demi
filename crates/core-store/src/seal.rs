//! Sealed credentials: a typed JSON document encrypted with AES-256-GCM under the core's key,
//! bound to the row it belongs to.
//!
//! It is stored as one BLOB of a random 12-byte nonce, the ciphertext and the 16-byte tag.
//!
//! The plugins' secrets are sealed here too (`crates/plugin-host/src/secrets.rs`), through the
//! product's side of the plugin host's port, and so are the values of the users' credentials.
//!
//! # The core's secret
//!
//! The key derives from the core's secret, 32 random bytes, with HKDF-SHA256 under a label of
//! its own, so no other key derived from the secret can stand in for it. A server's cores share
//! its instance secret (`crates/backend/src/config/secret.rs`); a core a program opens keeps its
//! own in its directory, so the directory alone reads its credentials.
//!
//! Sealing protects credentials when the database leaks alone; it is not key management, and
//! whoever holds both the database and the secret opens every credential.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use demi_shared_artifacts::{Mode, Permissions, Publication};
use demi_web_api_protocol::ids::{CredentialId, ProviderId, UserId};
use hkdf::Hkdf;
use sha2::Sha256;

const NONCE_BYTES: usize = 12;
const VAULT_LABEL: &[u8] = b"demi provider vault";

/// The secret a core's keys derive from. `Debug` never shows it.
#[derive(Clone)]
pub struct CoreSecret([u8; 32]);

/// Why a core's secret is unusable. No error shows the secret itself.
#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("the secret must be 64 hexadecimal digits")]
    Malformed,
    #[error("the secret file {} is not 64 hexadecimal digits", path.display())]
    Corrupt { path: PathBuf },
    #[error("the secret file {} cannot be read: {source}", path.display())]
    Read { path: PathBuf, source: io::Error },
    #[error("the secret file {} cannot be created: {source}", path.display())]
    Create {
        path: PathBuf,
        source: demi_shared_artifacts::Error,
    },
}

impl FromStr for CoreSecret {
    type Err = SecretError;

    fn from_str(text: &str) -> Result<Self, SecretError> {
        let mut bytes = [0; 32];
        hex::decode_to_slice(text, &mut bytes).map_err(|_| SecretError::Malformed)?;
        Ok(Self(bytes))
    }
}

impl fmt::Debug for CoreSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CoreSecret(..)")
    }
}

impl CoreSecret {
    /// The secret the file `path` holds as 64 hexadecimal digits, which the
    /// first read creates readable only by its owner.
    pub async fn load_or_create(path: &Path) -> Result<Self, SecretError> {
        match tokio::fs::read_to_string(path).await {
            Ok(text) => text.trim_end().parse().map_err(|_| SecretError::Corrupt {
                path: path.to_owned(),
            }),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let secret = Self(rand::random());
                let text = format!("{}\n", hex::encode(secret.0));
                let publication = Publication {
                    mode: Mode::CreateNew,
                    permissions: Permissions::Private,
                    durable: true,
                };
                demi_shared_artifacts::publish_bytes(path, text.as_bytes(), publication)
                    .await
                    .map_err(|source| SecretError::Create {
                        path: path.to_owned(),
                        source,
                    })?;
                Ok(secret)
            }
            Err(source) => Err(SecretError::Read {
                path: path.to_owned(),
                source,
            }),
        }
    }

    /// The key the core's credentials are sealed under.
    pub fn vault_key(&self) -> VaultKey {
        VaultKey::new(self.subkey(VAULT_LABEL))
    }

    /// The HKDF-SHA256 subkey under `label`, which no other label shares.
    pub fn subkey(&self, label: &[u8]) -> [u8; 32] {
        let mut key = [0; 32];
        Hkdf::<Sha256>::new(None, &self.0)
            .expand(label, &mut key)
            .expect("32 bytes is a valid HKDF-SHA256 output length");
        key
    }
}

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
    /// A schedule's URL secret, by the schedule's id.
    ScheduleSecret(&'a str),
    /// A URL secret as releases before schedules sealed it, by the id its
    /// automation had, which the schedule keeps. Only the migration that
    /// seals it again as a schedule's opens it.
    AutomationSecret(&'a str),
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
            Self::ScheduleSecret(schedule) => (b"demi schedule secret", vec![*schedule]),
            Self::AutomationSecret(automation) => (b"demi automation secret", vec![*automation]),
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
/// or sealed under another core's secret. Nothing replaces it.
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

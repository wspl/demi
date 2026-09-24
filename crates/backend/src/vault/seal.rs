//! Sealed credentials (`storage.md` § Passwords and credentials at rest): a
//! typed JSON document encrypted with AES-256-GCM under a key derived from
//! the instance secret, bound to the row it belongs to, and stored as one
//! BLOB of a random 12-byte nonce, the ciphertext and the 16-byte tag.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use demi_web_api::ids::{CredentialId, ProviderId};

const NONCE_BYTES: usize = 12;

/// The key credentials are sealed under.
#[derive(Clone)]
pub(crate) struct VaultKey(Aes256Gcm);

/// The row a sealed value belongs to: its additional authenticated data, so
/// that a value copied into another row does not open.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Row<'a> {
    /// An API-key entry's configuration.
    Config(&'a ProviderId),
    /// A subscription account's secret document.
    Secret(&'a ProviderId, &'a CredentialId),
}

impl Row<'_> {
    /// The row's name as bytes: a label, then each id preceded by its length,
    /// so that no two rows share a name.
    fn name(&self) -> Vec<u8> {
        let (label, ids): (&[u8], Vec<&str>) = match self {
            Self::Config(provider) => (b"demi provider config", vec![provider.as_str()]),
            Self::Secret(provider, credential) => {
                (b"demi account secret", vec![provider.as_str(), credential.as_str()])
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
pub(crate) struct Unsealable;

impl VaultKey {
    pub(crate) fn new(key: [u8; 32]) -> Self {
        Self(Aes256Gcm::new(&key.into()))
    }

    /// `plaintext` sealed for `row`.
    pub(crate) fn seal(&self, row: Row<'_>, plaintext: &[u8]) -> Vec<u8> {
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
    pub(crate) fn open(&self, row: Row<'_>, sealed: &[u8]) -> Result<Vec<u8>, Unsealable> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(id: &str) -> ProviderId {
        ProviderId::try_from(id).unwrap()
    }

    #[test]
    fn a_sealed_value_opens_only_for_its_row_under_its_key() {
        let key = VaultKey::new([7; 32]);
        let entry = provider("entry-1");
        let sealed = key.seal(Row::Config(&entry), br#"{"apiKey":"sk-test-123"}"#);
        assert!(!sealed.windows(11).any(|window| window == b"sk-test-123"));
        assert_eq!(
            key.open(Row::Config(&entry), &sealed).unwrap(),
            br#"{"apiKey":"sk-test-123"}"#
        );
        // A fresh nonce every time.
        assert_ne!(
            key.seal(Row::Config(&entry), b"same"),
            key.seal(Row::Config(&entry), b"same")
        );

        let other = provider("entry-2");
        assert_eq!(key.open(Row::Config(&other), &sealed), Err(Unsealable));
        let account = CredentialId::try_from("cred-1").unwrap();
        assert_eq!(key.open(Row::Secret(&entry, &account), &sealed), Err(Unsealable));
        assert_eq!(
            VaultKey::new([8; 32]).open(Row::Config(&entry), &sealed),
            Err(Unsealable)
        );
        let mut tampered = sealed.clone();
        *tampered.last_mut().unwrap() ^= 1;
        assert_eq!(key.open(Row::Config(&entry), &tampered), Err(Unsealable));
        assert_eq!(key.open(Row::Config(&entry), &sealed[..8]), Err(Unsealable));

        // Moving characters from one id to the other names another row.
        let (long, short) = (provider("ab"), CredentialId::try_from("c").unwrap());
        let (shorter, longer) = (provider("a"), CredentialId::try_from("bc").unwrap());
        let sealed = key.seal(Row::Secret(&long, &short), b"x");
        assert_eq!(key.open(Row::Secret(&shorter, &longer), &sealed), Err(Unsealable));
    }
}

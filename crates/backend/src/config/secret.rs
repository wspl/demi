//! The instance secret and the keys derived from it.
//!
//! The secret is 32 random bytes:
//!
//! - `DEMI_INSTANCE_SECRET`, when that is set.
//! - Otherwise the data directory's `instance-secret` file, which the first
//!   start creates readable only by its owner.
//!
//! It is the secret of every user's core (`crates/core-store/src/seal.rs`),
//! and the server's own keys derive from it under labels of their own. A
//! deployment sets `DEMI_INSTANCE_SECRET` to keep the secret apart from the
//! data it protects, or when several backends must share it.

use std::path::Path;
use std::str::FromStr;

use demi_backend_accounts::email_change::CodeKey;
use demi_backend_accounts::push::PushKey;
use demi_core_store::seal::{CoreSecret, VaultKey};

pub use demi_core_store::seal::SecretError;

const FILE: &str = "instance-secret";
const EMAIL_CODE_LABEL: &[u8] = b"demi email-change code";
const PUSH_LABEL: &str = "demi web push vapid";

/// The instance secret. `Debug` never shows it.
#[derive(Clone, Debug)]
pub struct InstanceSecret(CoreSecret);

impl FromStr for InstanceSecret {
    type Err = SecretError;

    fn from_str(text: &str) -> Result<Self, SecretError> {
        text.parse().map(Self)
    }
}

impl InstanceSecret {
    /// The data directory's secret, created on first use.
    pub(crate) async fn load_or_create(data_dir: &Path) -> Result<Self, SecretError> {
        CoreSecret::load_or_create(&data_dir.join(FILE)).await.map(Self)
    }

    /// The key email-change codes are hashed under.
    pub(crate) fn email_code_key(&self) -> CodeKey {
        CodeKey::new(self.0.subkey(EMAIL_CODE_LABEL))
    }

    /// The key every user's core seals its credentials under.
    pub(crate) fn vault_key(&self) -> VaultKey {
        self.0.vault_key()
    }

    /// The VAPID key push messages are signed with: the same for as long as the
    /// secret is, so the browsers' subscriptions outlive a restart. A subkey
    /// that is no P-256 scalar, one in about four billion, gives way to the
    /// next label's.
    pub(crate) fn push_key(&self) -> PushKey {
        (0u32..)
            .find_map(|attempt| {
                let label = match attempt {
                    0 => PUSH_LABEL.to_owned(),
                    n => format!("{PUSH_LABEL} {n}"),
                };
                PushKey::new(self.0.subkey(label.as_bytes()))
            })
            .expect("some subkey is a P-256 scalar")
    }
}

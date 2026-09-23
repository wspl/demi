//! The instance secret (`storage.md` § Passwords and credentials at rest):
//! 32 random bytes, `DEMI_INSTANCE_SECRET` when that is set, otherwise the
//! data directory's `instance-secret` file, which the first start creates
//! readable only by its owner. Each key the backend needs derives from it
//! with HKDF-SHA256 under a label of its own.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use demi_artifact::{Mode, Permissions, Publication};
use hkdf::Hkdf;
use sha2::Sha256;

const FILE: &str = "instance-secret";
const EMAIL_CODE_LABEL: &[u8] = b"demi email-change code";

/// The instance secret. `Debug` never shows it.
#[derive(Clone)]
pub struct InstanceSecret([u8; 32]);

/// Why the instance secret is unusable. A malformed secret stops startup,
/// and no error shows the secret itself.
#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("the instance secret must be 64 hexadecimal digits")]
    Malformed,
    #[error("the instance secret file {} is not 64 hexadecimal digits", path.display())]
    Corrupt { path: PathBuf },
    #[error("the instance secret file {} cannot be read: {source}", path.display())]
    Read { path: PathBuf, source: io::Error },
    #[error("the instance secret file {} cannot be created: {source}", path.display())]
    Create {
        path: PathBuf,
        source: demi_artifact::Error,
    },
}

impl FromStr for InstanceSecret {
    type Err = SecretError;

    fn from_str(text: &str) -> Result<Self, SecretError> {
        let mut bytes = [0; 32];
        hex::decode_to_slice(text, &mut bytes).map_err(|_| SecretError::Malformed)?;
        Ok(Self(bytes))
    }
}

impl fmt::Debug for InstanceSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("InstanceSecret(..)")
    }
}

impl InstanceSecret {
    /// The data directory's secret, created on first use.
    pub(crate) async fn load_or_create(data_dir: &Path) -> Result<Self, SecretError> {
        let path = data_dir.join(FILE);
        match tokio::fs::read_to_string(&path).await {
            Ok(text) => text.trim_end().parse().map_err(|_| SecretError::Corrupt { path }),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let secret = Self(rand::random());
                let text = format!("{}\n", hex::encode(secret.0));
                let publication = Publication {
                    mode: Mode::CreateNew,
                    permissions: Permissions::Private,
                    durable: true,
                };
                demi_artifact::publish_bytes(&path, text.as_bytes(), publication)
                    .await
                    .map_err(|source| SecretError::Create { path, source })?;
                Ok(secret)
            }
            Err(source) => Err(SecretError::Read { path, source }),
        }
    }

    /// The key email-change codes are hashed under.
    pub(crate) fn email_code_key(&self) -> CodeKey {
        let mut key = [0; 32];
        Hkdf::<Sha256>::new(None, &self.0)
            .expand(EMAIL_CODE_LABEL, &mut key)
            .expect("32 bytes is a valid HKDF-SHA256 output length");
        CodeKey(key)
    }
}

/// The key of email-change code hashes.
#[derive(Clone)]
pub(crate) struct CodeKey([u8; 32]);

impl CodeKey {
    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt as _;

    use super::*;

    #[tokio::test]
    async fn the_secret_file_is_created_once_readable_by_its_owner_only() {
        let data = tempfile::tempdir().unwrap();
        let first = InstanceSecret::load_or_create(data.path()).await.unwrap();
        let mode = std::fs::metadata(data.path().join(FILE)).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        let again = InstanceSecret::load_or_create(data.path()).await.unwrap();
        assert_eq!(first.0, again.0);

        std::fs::write(data.path().join(FILE), "not hex\n").unwrap();
        let corrupt = InstanceSecret::load_or_create(data.path()).await.unwrap_err();
        assert!(matches!(corrupt, SecretError::Corrupt { .. }), "{corrupt}");
    }

    #[test]
    fn a_configured_secret_is_64_hex_digits() {
        let digits = "0123456789abcdef".repeat(4);
        assert!(digits.parse::<InstanceSecret>().is_ok());
        assert!(digits.to_uppercase().parse::<InstanceSecret>().is_ok());
        assert!(digits[1..].parse::<InstanceSecret>().is_err());
        assert!(format!("{}zz", &digits[2..]).parse::<InstanceSecret>().is_err());
    }
}

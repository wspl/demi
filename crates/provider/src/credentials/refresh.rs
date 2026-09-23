//! The token refresh every family follows (`providers.md` § Token refresh):
//! one refresh at a time per account in this process, a fresh read after the
//! wait, and a versioned replace, so that refreshers of one account, in this
//! backend or another worker, never invalidate each other's tokens.

use std::future::Future;

use demi_gates::{KeyedPermit, KeyedSerialGate};
use serde::{Serialize, de::DeserializeOwned};

use crate::credentials::{AccountDocument, PoolError};

/// One refresh at a time for each account in this process, first come first
/// served. Its owner keys accounts so that no two entries' accounts share a
/// key, such as by the provider entry's id and the account's id.
#[derive(Debug, Default)]
pub struct RefreshGates(KeyedSerialGate<String>);

impl RefreshGates {
    pub fn new() -> Self {
        Self::default()
    }

    /// Waits for the account's turn. Dropping the future gives up its place.
    pub async fn turn(&self, account: &str) -> RefreshPermit {
        RefreshPermit(self.0.acquire(account.to_owned()).await)
    }
}

/// An account's refresh turn; dropping it gives the next refresher its turn.
#[derive(Debug)]
pub struct RefreshPermit(#[allow(dead_code, reason = "held for its drop")] KeyedPermit<String>);

/// A family's secret document, as its provider crate defines it: decoded
/// strictly from the stored text, a corrupt one refused, and encoded back
/// after a refresh.
pub trait SecretDocument: Serialize + DeserializeOwned {
    fn decode(text: &str) -> Result<Self, SecretDecodeError> {
        decode_secret(text)
    }

    fn encode(&self) -> String {
        // A secret document is strings, numbers and times, which always
        // serialize.
        serde_json::to_string(self).expect("a secret document serializes")
    }
}

/// Why JSON that holds secrets, a stored secret document or a vendor's token
/// response, could not be decoded: where, and what kind of failure. It never
/// quotes the text, whose values may be tokens.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("malformed at {path}: {fault}")]
pub struct SecretDecodeError {
    /// The path of the field that failed, such as `refreshToken`; `.` for the
    /// document itself and for text that is not JSON.
    pub path: String,
    pub fault: SecretFault,
}

/// The kind of a [`SecretDecodeError`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SecretFault {
    #[error("not JSON")]
    Syntax,
    #[error("a field is missing, unknown or of the wrong type")]
    Shape,
}

/// Decodes JSON text that holds secrets into `T`, reporting a failure by the
/// field's path and its kind only: serde's own message quotes the offending
/// value, which may be a token.
pub fn decode_secret<T: DeserializeOwned>(text: &str) -> Result<T, SecretDecodeError> {
    let mut json = serde_json::Deserializer::from_str(text);
    let decoded = serde_path_to_error::deserialize(&mut json).map_err(|error| {
        let fault = fault(error.inner());
        // Text that is not JSON has no field to name.
        let path = match fault {
            SecretFault::Syntax => ".".to_owned(),
            SecretFault::Shape => error.path().to_string(),
        };
        SecretDecodeError { path, fault }
    })?;
    json.end().map_err(|error| SecretDecodeError {
        path: ".".into(),
        fault: fault(&error),
    })?;
    Ok(decoded)
}

fn fault(error: &serde_json::Error) -> SecretFault {
    match error.classify() {
        serde_json::error::Category::Data => SecretFault::Shape,
        serde_json::error::Category::Syntax
        | serde_json::error::Category::Eof
        | serde_json::error::Category::Io => SecretFault::Syntax,
    }
}

/// Why an account's secret could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AccountError {
    #[error(transparent)]
    Store(PoolError),
    /// The account has no secret document, such as one removed meanwhile.
    #[error("the account has no secret document")]
    Missing,
    /// The stored document is corrupt; it is refused, never repaired.
    #[error("the account's secret document is {0}")]
    Invalid(SecretDecodeError),
}

/// Why a renewal gave no usable secret.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RenewError<E> {
    #[error(transparent)]
    Account(AccountError),
    /// The vendor refused the refresh and nobody else stored new tokens.
    #[error(transparent)]
    Refresh(E),
}

impl<E> From<AccountError> for RenewError<E> {
    fn from(error: AccountError) -> Self {
        Self::Account(error)
    }
}

/// A decoded secret document and the version it was read at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stored<S> {
    pub secret: S,
    pub version: u64,
}

/// Reads and decodes an account's secret document.
pub async fn read_secret<S: SecretDocument>(doc: &dyn AccountDocument) -> Result<Stored<S>, AccountError> {
    let revision = doc.read().await.map_err(AccountError::Store)?;
    let Some(revision) = revision else {
        return Err(AccountError::Missing);
    };
    let secret = S::decode(&revision.text).map_err(AccountError::Invalid)?;
    Ok(Stored {
        secret,
        version: revision.version,
    })
}

/// An account's secret, refreshed by the one protocol of `providers.md` §
/// Token refresh when `due` says it needs a refresh: take the account's
/// turn; read the stored document again and use it when `due` no longer
/// says so, because another refresher stored new tokens meanwhile;
/// otherwise ask the vendor through `refresh`. A refreshed secret is stored
/// over the version read, and when another writer stored first, its secret
/// is used instead. A refusal uses the tokens another writer stored
/// meanwhile, and fails only when nobody did. A secret that is not due is
/// used without taking the turn. The turn ends however this returns,
/// cancellation included.
pub async fn renew<S, E, F, R>(
    doc: &dyn AccountDocument,
    due: impl Fn(&S) -> bool,
    refresh: F,
) -> Result<S, RenewError<E>>
where
    S: SecretDocument,
    F: FnOnce(S) -> R,
    R: Future<Output = Result<S, E>>,
{
    let stored = read_secret::<S>(doc).await?;
    if !due(&stored.secret) {
        return Ok(stored.secret);
    }
    let _turn = doc.refresh_turn().await;
    let latest = read_secret::<S>(doc).await?;
    if !due(&latest.secret) {
        return Ok(latest.secret);
    }
    let version = latest.version;
    match refresh(latest.secret).await {
        Err(error) => {
            let stored = read_secret::<S>(doc).await?;
            if stored.version == version {
                return Err(RenewError::Refresh(error));
            }
            Ok(stored.secret)
        }
        Ok(next) => {
            let kept = doc
                .replace(next.encode(), version)
                .await
                .map_err(AccountError::Store)?;
            if kept {
                return Ok(next);
            }
            Ok(read_secret::<S>(doc).await?.secret)
        }
    }
}

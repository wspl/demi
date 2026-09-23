//! Password hashing with argon2id (`storage.md` § Passwords and credentials
//! at rest).

use std::fmt;
use std::num::NonZeroUsize;
use std::sync::Arc;

use argon2::Argon2;
use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{self, PasswordHasher as _, PasswordVerifier as _, SaltString};
use demi_web_api::auth::Password;
use tokio::sync::Semaphore;

/// A stored password hash: a PHC string such as
/// `$argon2id$v=19$m=19456,t=2,p=1$<salt>$<hash>`, which records its
/// algorithm and parameters beside the salt and the hash.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct PasswordHash(String);

impl PasswordHash {
    /// A PHC string read back from storage.
    pub(crate) fn parse(text: String) -> Result<Self, password_hash::Error> {
        password_hash::PasswordHash::new(&text)?;
        Ok(Self(text))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for PasswordHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PasswordHash(..)")
    }
}

/// Why hashing or verifying failed.
#[derive(Debug, thiserror::Error)]
pub enum HashError {
    #[error("argon2 failed: {0}")]
    Argon2(password_hash::Error),
    #[error("the hashing task failed: {0}")]
    Task(#[from] tokio::task::JoinError),
}

/// Hashes and verifies passwords with the argon2 crate's default parameters,
/// on the blocking pool and no more at once than the machine has CPUs,
/// because each hash holds its memory until it finishes.
#[derive(Clone)]
pub(crate) struct PasswordHasher {
    permits: Arc<Semaphore>,
    /// What a login for an address without an account verifies against. It
    /// is made like every other hash, so both verifications take as long.
    dummy: PasswordHash,
}

impl PasswordHasher {
    pub(crate) async fn new() -> Result<Self, HashError> {
        let parallelism = std::thread::available_parallelism().map_or(1, NonZeroUsize::get);
        let permits = Arc::new(Semaphore::new(parallelism));
        let unguessable = Password::from(uuid::Uuid::new_v4().to_string());
        let dummy = run(&permits, move || hash(&unguessable)).await?;
        Ok(Self { permits, dummy })
    }

    pub(crate) async fn hash(&self, password: Password) -> Result<PasswordHash, HashError> {
        run(&self.permits, move || hash(&password)).await
    }

    /// Whether `password` matches `stored`. Without a stored hash, as for an
    /// address without an account, it verifies against the dummy hash and
    /// answers false, so the answer takes as long either way.
    pub(crate) async fn verify(&self, password: Password, stored: Option<PasswordHash>) -> Result<bool, HashError> {
        let known = stored.is_some();
        let against = stored.unwrap_or_else(|| self.dummy.clone());
        let matches = run(&self.permits, move || verify(&password, &against)).await?;
        Ok(known && matches)
    }
}

async fn run<T: Send + 'static>(
    permits: &Arc<Semaphore>,
    work: impl FnOnce() -> Result<T, HashError> + Send + 'static,
) -> Result<T, HashError> {
    let permit = permits
        .clone()
        .acquire_owned()
        .await
        .expect("the hashing semaphore is never closed");
    // The permit moves into the blocking task: a requester that goes away
    // leaves the hash running, and its memory counted, until it finishes.
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        work()
    })
    .await?
}

fn hash(password: &Password) -> Result<PasswordHash, HashError> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.expose().as_bytes(), &salt)
        .map(|hash| PasswordHash(hash.to_string()))
        .map_err(HashError::Argon2)
}

fn verify(password: &Password, stored: &PasswordHash) -> Result<bool, HashError> {
    let parsed = password_hash::PasswordHash::new(stored.as_str()).map_err(HashError::Argon2)?;
    match Argon2::default().verify_password(password.expose().as_bytes(), &parsed) {
        Ok(()) => Ok(true),
        Err(password_hash::Error::Password) => Ok(false),
        Err(error) => Err(HashError::Argon2(error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_password_verifies_against_its_own_hash_only() {
        let hasher = PasswordHasher::new().await.unwrap();
        let stored = hasher.hash(Password::from("right-pass-1".to_owned())).await.unwrap();
        assert!(stored.as_str().starts_with("$argon2id$v=19$m=19456,t=2,p=1$"));
        let right = Password::from("right-pass-1".to_owned());
        assert!(hasher.verify(right.clone(), Some(stored.clone())).await.unwrap());
        let wrong = Password::from("wrong-pass-1".to_owned());
        assert!(!hasher.verify(wrong, Some(stored)).await.unwrap());
        assert!(!hasher.verify(right, None).await.unwrap());
    }
}

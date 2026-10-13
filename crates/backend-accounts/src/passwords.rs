//! Password hashing with argon2id (`storage.md` § Passwords and credentials
//! at rest).

use std::num::NonZeroUsize;
use std::sync::Arc;

use argon2::Argon2;
use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{self, PasswordHasher as _, PasswordVerifier as _, SaltString};
use demi_web_api_protocol::auth::Password;
use tokio::sync::Semaphore;

use demi_backend_database::accounts::PasswordHash;

/// Why hashing or verifying failed.
#[derive(Debug, thiserror::Error)]
pub enum HashError {
    #[error("argon2 failed: {0}")]
    Argon2(password_hash::Error),
    #[error("the hashing task failed: {0}")]
    Task(#[from] tokio::task::JoinError),
}

/// How many characters a first password has: letters and digits, about 119
/// bits, which its owner replaces at once.
const FIRST_PASSWORD_LENGTH: usize = 20;

/// A first password for an account an administrator made or reset
/// (`product.md` § User system): 20 random letters and digits, never typed
/// by anyone.
pub fn generate_first_password() -> Password {
    use rand::distr::{Alphanumeric, SampleString as _};
    Password::from(Alphanumeric.sample_string(&mut rand::rng(), FIRST_PASSWORD_LENGTH))
}

/// Hashes and verifies passwords with the argon2 crate's default parameters,
/// on the blocking pool and no more at once than the machine has CPUs,
/// because each hash holds its memory until it finishes.
#[derive(Clone)]
pub struct PasswordHasher {
    permits: Arc<Semaphore>,
    /// What a login for an address without an account verifies against. It
    /// is made like every other hash, so both verifications take as long.
    dummy: PasswordHash,
}

impl PasswordHasher {
    pub async fn new() -> Result<Self, HashError> {
        let parallelism = std::thread::available_parallelism().map_or(1, NonZeroUsize::get);
        let permits = Arc::new(Semaphore::new(parallelism));
        let unguessable = Password::from(uuid::Uuid::new_v4().to_string());
        let dummy = run(&permits, move || hash(&unguessable)).await?;
        Ok(Self { permits, dummy })
    }

    pub async fn hash(&self, password: Password) -> Result<PasswordHash, HashError> {
        run(&self.permits, move || hash(&password)).await
    }

    /// Whether `password` matches `stored`. Without a stored hash, as for an
    /// address without an account, it verifies against the dummy hash and
    /// answers false, so the answer takes as long either way.
    pub async fn verify(
        &self,
        password: Password,
        stored: Option<PasswordHash>,
    ) -> Result<bool, HashError> {
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
        .map(|hash| PasswordHash::from(&hash))
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

    #[test]
    fn a_first_password_is_twenty_letters_and_digits_and_never_repeats() {
        let first = generate_first_password();
        assert_eq!(first.expose().chars().count(), 20);
        assert!(first.expose().chars().all(|c| c.is_ascii_alphanumeric()));
        assert_ne!(first.expose(), generate_first_password().expose());
    }
}

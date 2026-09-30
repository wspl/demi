//! Changing an account's email address (`web-api.md` § Account API): the
//! current password starts a challenge, and only the six-digit code sent to
//! the new address changes the login identity.

use std::sync::Arc;

use demi_backend_storage::StorageError;
use demi_backend_storage::accounts::{ChallengeIssue, ChallengeOutcome, ChallengePolicy, CodeHash};
use demi_backend_storage::control::ControlService;
use demi_core::Timestamp;
use demi_web_api::auth::{EmailChallengeDto, Password};
use demi_web_api::ids::UserId;
use demi_web_api::text::EmailAddress;
use futures_util::future::BoxFuture;
use jiff::SignedDuration;
use rand::Rng;

use crate::passwords::{HashError, PasswordHasher};

/// Delivers verification codes. A deployment supplies it; tests capture the
/// mail instead of sending it.
pub trait AccountMail: Send + Sync + 'static {
    fn send_verification(&self, mail: VerificationMail) -> BoxFuture<'static, Result<(), MailError>>;
}

/// A verification code on its way to the address it proves.
#[derive(Debug, Clone)]
pub struct VerificationMail {
    pub email: EmailAddress,
    pub code: String,
    pub expires_at: Timestamp,
}

/// Why a verification mail was not delivered.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct MailError(pub String);

const CHALLENGE_POLICY: ChallengePolicy = ChallengePolicy {
    lifetime: SignedDuration::from_mins(10),
    cooldown: SignedDuration::from_mins(1),
    attempts: 5,
};

/// The key email-change codes are hashed under, which the instance secret
/// derives (`storage.md` § Passwords and credentials at rest).
#[derive(Clone)]
pub struct CodeKey([u8; 32]);

impl CodeKey {
    pub fn new(key: [u8; 32]) -> Self {
        Self(key)
    }
}

/// Why a start was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartRefusal {
    MailUnavailable,
    InvalidCredentials,
    EmailTaken,
    /// The last code went out less than the cooldown ago.
    CoolingDown,
    MailFailed,
}

/// A start either issues a challenge or is refused.
pub enum StartOutcome {
    Issued(EmailChallengeDto),
    Refused(StartRefusal),
}

#[derive(Debug, thiserror::Error)]
pub enum EmailChangeError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error(transparent)]
    Hash(#[from] HashError),
}

pub struct EmailChanges {
    control: ControlService,
    hasher: PasswordHasher,
    mail: Option<Arc<dyn AccountMail>>,
    key: CodeKey,
}

impl EmailChanges {
    pub fn new(
        control: ControlService,
        hasher: PasswordHasher,
        mail: Option<Arc<dyn AccountMail>>,
        key: CodeKey,
    ) -> Self {
        Self {
            control,
            hasher,
            mail,
            key,
        }
    }

    /// Checks the current password and sends a code to `email`.
    pub async fn start(
        &self,
        user: UserId,
        email: EmailAddress,
        password: Password,
    ) -> Result<StartOutcome, EmailChangeError> {
        let Some(mail) = &self.mail else {
            return Ok(StartOutcome::Refused(StartRefusal::MailUnavailable));
        };
        let Some(account) = self.control.account(user.clone()).await? else {
            return Ok(StartOutcome::Refused(StartRefusal::InvalidCredentials));
        };
        let password_hash = account.password_hash;
        if !self.hasher.verify(password, Some(password_hash.clone())).await? {
            return Ok(StartOutcome::Refused(StartRefusal::InvalidCredentials));
        }
        if self.control.email_in_use(email.clone()).await? {
            return Ok(StartOutcome::Refused(StartRefusal::EmailTaken));
        }
        let id = uuid::Uuid::new_v4().to_string();
        let code = format!("{:06}", rand::rng().random_range(0..1_000_000));
        let issue = ChallengeIssue {
            user: user.clone(),
            id: id.clone(),
            email: email.clone(),
            password_hash,
            code_hash: CodeHash::of(&self.key.0, &id, &code),
        };
        let Some(expires_at) = self.control.issue_email_challenge(issue, CHALLENGE_POLICY).await? else {
            return Ok(StartOutcome::Refused(StartRefusal::CoolingDown));
        };
        let verification = VerificationMail {
            email: email.clone(),
            code,
            expires_at,
        };
        if let Err(error) = mail.send_verification(verification).await {
            tracing::warn!(error = &error as &dyn std::error::Error, "a verification mail was not delivered");
            // A code that never arrived must not stay usable or hold back
            // the retry.
            self.control.delete_email_challenge(user, id).await?;
            return Ok(StartOutcome::Refused(StartRefusal::MailFailed));
        }
        Ok(StartOutcome::Issued(EmailChallengeDto { id, email, expires_at }))
    }

    /// Changes the address when `code` is the one the challenge sent.
    pub async fn confirm(
        &self,
        user: UserId,
        challenge: String,
        code: &str,
    ) -> Result<ChallengeOutcome, StorageError> {
        let code_hash = CodeHash::of(&self.key.0, &challenge, code);
        self.control
            .confirm_email_challenge(user, challenge, code_hash, CHALLENGE_POLICY.attempts)
            .await
    }
}

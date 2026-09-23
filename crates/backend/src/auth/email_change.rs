//! Changing an account's email address (`web-api.md` § Account API): the
//! current password starts a challenge, and only the six-digit code sent to
//! the new address changes the login identity.

use std::sync::Arc;

use demi_core::Timestamp;
use demi_web_api::auth::{EmailChallengeDto, Password, UserDto};
use demi_web_api::ids::UserId;
use demi_web_api::text::EmailAddress;
use futures_util::future::BoxFuture;
use hmac::{Hmac, Mac};
use jiff::SignedDuration;
use rand::Rng;
use sha2::Sha256;
use subtle::ConstantTimeEq;

use crate::auth::passwords::{HashError, PasswordHash, PasswordHasher};
use crate::storage::StorageError;
use crate::storage::control::ControlService;
use crate::vault::secret::CodeKey;

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

/// How long a code lasts, how soon another may be sent, and how many wrong
/// codes a challenge takes.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ChallengePolicy {
    pub(crate) lifetime: SignedDuration,
    pub(crate) cooldown: SignedDuration,
    pub(crate) attempts: u32,
}

const CHALLENGE_POLICY: ChallengePolicy = ChallengePolicy {
    lifetime: SignedDuration::from_mins(10),
    cooldown: SignedDuration::from_mins(1),
    attempts: 5,
};

/// The HMAC-SHA256 of a challenge's id and code in lowercase hexadecimal,
/// under the email-change key: what storage keeps instead of the code.
#[derive(Debug, Clone)]
pub(crate) struct CodeHash(String);

impl CodeHash {
    fn of(key: &CodeKey, challenge: &str, code: &str) -> Self {
        let mut mac = Hmac::<Sha256>::new_from_slice(key.as_bytes()).expect("HMAC takes a key of any length");
        mac.update(format!("email-change:{challenge}:{code}").as_bytes());
        Self(hex::encode(mac.finalize().into_bytes()))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    /// Compares in constant time, so the comparison does not reveal how much
    /// of a stored hash a guess matched.
    pub(crate) fn matches(&self, stored: &str) -> bool {
        self.0.as_bytes().ct_eq(stored.as_bytes()).into()
    }
}

/// A challenge to store: the new address, and the password hash it was
/// issued under, so a password change ends it.
pub(crate) struct ChallengeIssue {
    pub(crate) user: UserId,
    pub(crate) id: String,
    pub(crate) email: EmailAddress,
    pub(crate) password_hash: PasswordHash,
    pub(crate) code_hash: CodeHash,
}

/// How a confirmation ended.
pub(crate) enum ChallengeOutcome {
    /// The account has the new address; the challenge is consumed.
    Changed(UserDto),
    /// The code is wrong, expired or used up, or the challenge no longer
    /// holds.
    InvalidCode,
    /// Another account took the address after the challenge was issued.
    EmailTaken,
}

/// Why a start was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StartRefusal {
    MailUnavailable,
    InvalidCredentials,
    EmailTaken,
    /// The last code went out less than the cooldown ago.
    CoolingDown,
    MailFailed,
}

/// A start either issues a challenge or is refused.
pub(crate) enum StartOutcome {
    Issued(EmailChallengeDto),
    Refused(StartRefusal),
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum EmailChangeError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error(transparent)]
    Hash(#[from] HashError),
}

pub(crate) struct EmailChanges {
    control: ControlService,
    hasher: PasswordHasher,
    mail: Option<Arc<dyn AccountMail>>,
    key: CodeKey,
}

impl EmailChanges {
    pub(crate) fn new(
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
    pub(crate) async fn start(
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
            code_hash: CodeHash::of(&self.key, &id, &code),
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
    pub(crate) async fn confirm(
        &self,
        user: UserId,
        challenge: String,
        code: &str,
    ) -> Result<ChallengeOutcome, StorageError> {
        let code_hash = CodeHash::of(&self.key, &challenge, code);
        self.control
            .confirm_email_challenge(user, challenge, code_hash, CHALLENGE_POLICY.attempts)
            .await
    }
}

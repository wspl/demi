//! A Claude Code account: its record holds only its public metadata, since
//! the CLI keeps its sign-in in the account's directory on the Claude Code
//! machine and renews it there. The provider names the account a process uses,
//! marks it signed out when the vendor refused its sign-in, and hands over,
//! once, the sign-in an earlier release kept sealed in its record.
//!
//! - One directory per account is the CLI's configuration directory for every
//!   process of that account, and the one place its sign-in exists. That is a
//!   requirement: a renewal revokes at once the refresh token it spends and
//!   the access token it replaces (measured with Claude Code 2.1.301), so a
//!   second copy of a sign-in anywhere is signed out by the first renewal of
//!   the other. In one directory the CLI renews once for all its processes,
//!   under a lock there.
//! - A process is always closed, SIGTERM first, never killed: one killed while
//!   it renews leaves the lock behind, and for about a minute every process of
//!   the account fails, until the CLI takes the lock back (the sign-in
//!   survives). The backend never renews a Claude sign-in itself.
//! - A refused renewal ends a run with `authentication_failed` (`is_error`,
//!   though its `subtype` says success); the account is marked signed out
//!   until a login replaces it, and account rotation passes over it.
//! - Signing an account in again and removing it replace or delete its
//!   directory, so each first stops new processes of the account from starting,
//!   requests and probes alike, closes every process it has, and only then
//!   changes the directory.
//! - The handover moves the tokens an earlier release kept sealed into the
//!   directory and clears the record's secret, so the backend holds none; a
//!   directory that already holds a sign-in wins.

use std::sync::Arc;

use demi_provider_common::Secret;
use demi_provider_common::credentials::{
    AccountError, AccountLabel, AccountMeta, AuthFailure, AuthReason, CredentialPool,
    SecretDocument, read_secret,
};
use demi_shared_types::Timestamp;
use serde::{Deserialize, Serialize};

use crate::FAMILY;
use crate::files::SignIn;
use crate::placement::{CliWork, Handover};

/// The sign-in an earlier release kept sealed in an account's record: what
/// the CLI's login wrote, its tokens and their expiry, the scopes granted and
/// the subscription type, with the account's ID and email. It is read only to
/// hand it over to the account's directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EarlierSignIn {
    access_token: Secret,
    refresh_token: Secret,
    expires_at: Timestamp,
    scopes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    subscription_type: Option<String>,
    account_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    email: Option<String>,
}

impl SecretDocument for EarlierSignIn {}

/// How an account the CLI signed in names itself: its email, else its ID;
/// its ID tells it apart.
pub(crate) fn label(account_id: &str, email: Option<&str>) -> AccountLabel {
    AccountLabel {
        label: email.unwrap_or(account_id).to_owned(),
        detail: None,
        identity_key: Some(identity_key(account_id)),
    }
}

/// What tells the account of `account_id` apart from an entry's others.
pub(crate) fn identity_key(account_id: &str) -> String {
    format!("account:{account_id}")
}

/// The account a provider stands for.
pub(crate) struct ClaudeAuth {
    pool: Arc<dyn CredentialPool>,
    account: Option<String>,
}

/// The work of a process of the account, and the version of the record's
/// secret to clear once its handover is written.
pub(crate) struct AccountWork {
    pub(crate) work: CliWork,
    pub(crate) handed_over: Option<u64>,
}

impl ClaudeAuth {
    pub(crate) fn new(pool: Arc<dyn CredentialPool>, account: Option<String>) -> Self {
        Self { pool, account }
    }

    fn account(&self) -> Result<&str, AuthFailure> {
        self.account
            .as_deref()
            .ok_or(AuthFailure::new(FAMILY, AuthReason::Missing))
    }

    /// The account as its record holds it: a usable one, else why it is
    /// not, such as a sign-in its vendor refused.
    pub(crate) async fn status(&self) -> Result<AccountMeta, AuthFailure> {
        let account = self.account()?;
        let meta = self
            .pool
            .meta(account)
            .await
            .map_err(|error| AuthFailure::new(FAMILY, AuthReason::Store(error.to_string())))?
            .ok_or(AuthFailure::new(FAMILY, AuthReason::Missing))?;
        if meta.signed_out {
            return Err(AuthFailure::new(FAMILY, AuthReason::SignedOut));
        }
        Ok(meta)
    }

    /// What a process of the account starts with: the account's directory,
    /// and the sign-in its record still holds from an earlier release, as
    /// the CLI's files, when it holds one. A signed-out account starts
    /// nothing: the CLI emptied its sign-in.
    pub(crate) async fn work(&self) -> Result<AccountWork, AuthFailure> {
        self.status().await?;
        let account = self.account()?;
        let document = self.pool.document(account);
        let (handover, handed_over) = match read_secret::<EarlierSignIn>(&*document).await {
            Err(AccountError::Missing) => (None, None),
            Err(AccountError::Invalid(error)) => {
                return Err(AuthFailure::new(
                    FAMILY,
                    AuthReason::Invalid(format!(
                        "its secret document is {error}. Remove the account and sign in again"
                    )),
                ));
            }
            Err(error) => return Err(AuthFailure::of_account(FAMILY, error)),
            Ok(stored) => {
                let earlier = stored.secret;
                let files = SignIn {
                    access_token: &earlier.access_token,
                    refresh_token: &earlier.refresh_token,
                    expires_at: earlier.expires_at,
                    scopes: &earlier.scopes,
                    subscription_type: earlier.subscription_type.as_deref(),
                    account_id: &earlier.account_id,
                    email: earlier.email.as_deref(),
                }
                .files();
                (Some(Handover { files }), Some(stored.version))
            }
        };
        Ok(AccountWork {
            work: CliWork::Account {
                account: account.to_owned(),
                handover,
            },
            handed_over,
        })
    }

    /// Clears the record's secret, read at `version`, once its sign-in is in
    /// the account's directory: the tokens move, they are not copied. A
    /// record that changed meanwhile was written by a new sign-in, which
    /// holds no secret.
    pub(crate) async fn handed_over(&self, version: u64) {
        let Ok(account) = self.account() else {
            return;
        };
        let document = self.pool.document(account);
        match document.replace(None, version).await {
            Ok(_) => tracing::info!(account, "a Claude Code sign-in of an earlier release was handed over to its directory"),
            Err(error) => {
                // The next start finds the directory's sign-in and clears it
                // then, writing nothing.
                tracing::warn!(account, "a handed-over Claude Code sign-in was not cleared from its record: {error}");
            }
        }
    }

    /// Marks the account signed out: its vendor refused its sign-in. A pool
    /// that fails keeps the account as it was, and its next request fails
    /// the same way.
    pub(crate) async fn signed_out(&self) {
        let Ok(account) = self.account() else {
            return;
        };
        tracing::info!(account, "Claude signed a Claude Code account out");
        if let Err(error) = self.pool.sign_out(account).await {
            tracing::warn!(account, "a signed-out Claude Code account was not marked: {error}");
        }
    }
}

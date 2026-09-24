//! A Claude Code account (`providers.md` § Subscription secrets, § Login and
//! publication): its secret document, the setup token `claude setup-token`
//! printed, which is used as it is and never refreshed; how the account
//! names itself; and how an entry takes a new one.

use std::sync::Arc;

use demi_core::LoginPending;
use demi_provider::Secret;
use demi_provider::credentials::{
    AccountKit, AccountLabel, AccountsCapability, AccountsError, AddAccount, AuthFailure,
    AuthReason, CredentialPool, LoginError, NewAccount, SecretDocument, read_secret,
};
use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::FAMILY;

/// The `claude-code` family's secret document. Demi defines it; it is not the
/// CLI's own credentials file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ClaudeSecret {
    /// The setup token, which the CLI takes as `CLAUDE_CODE_OAUTH_TOKEN`.
    pub(crate) access_token: Secret,
}

impl SecretDocument for ClaudeSecret {}

impl ClaudeSecret {
    /// How the account names itself: a setup token says nothing about its
    /// account, so the account is known by a digest of the token. The first
    /// sixteen hexadecimal digits of its SHA-256 tell accounts apart, and the
    /// label shows the last eight of them.
    pub(crate) fn label(&self) -> AccountLabel {
        let digest = Sha256::digest(self.access_token.expose().as_bytes());
        let hex: String = digest[..8]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        AccountLabel {
            label: format!("claude-{}", &hex[8..]),
            detail: None,
            identity_key: Some(format!("token:{hex}")),
        }
    }
}

/// The account a provider stands for, read from the entry's pool.
pub(crate) struct ClaudeAuth {
    pool: Arc<dyn CredentialPool>,
    account: Option<String>,
}

impl ClaudeAuth {
    pub(crate) fn new(pool: Arc<dyn CredentialPool>, account: Option<String>) -> Self {
        Self { pool, account }
    }

    /// The account's stored secret: a provider without an account, or an
    /// account without a document, is signed out, and a corrupt document is
    /// refused.
    pub(crate) async fn stored(&self) -> Result<ClaudeSecret, AuthFailure> {
        let Some(account) = &self.account else {
            return Err(AuthFailure::new(FAMILY, AuthReason::Missing));
        };
        let document = self.pool.document(account);
        read_secret::<ClaudeSecret>(&*document)
            .await
            .map(|stored| stored.secret)
            .map_err(|error| AuthFailure::of_account(FAMILY, error))
    }
}

/// What the `claude-code` family adds to the account operations: an account
/// comes from a setup token; there is no device login.
pub(crate) struct ClaudeKit;

impl AccountKit for ClaudeKit {
    fn capability(&self) -> AccountsCapability {
        AccountsCapability {
            login: false,
            add: true,
        }
    }

    fn login<'a>(
        &'a self,
        _pending: &'a (dyn Fn(LoginPending) + Send + Sync),
    ) -> Option<BoxFuture<'a, Result<NewAccount, LoginError>>> {
        None
    }

    fn add(&self, input: AddAccount) -> Option<Result<NewAccount, AccountsError>> {
        let AddAccount::SetupToken(token) = input;
        let secret = ClaudeSecret {
            access_token: token,
        };
        Some(Ok(NewAccount {
            label: secret.label(),
            secret: secret.encode(),
        }))
    }
}

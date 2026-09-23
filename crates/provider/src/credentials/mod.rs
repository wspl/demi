//! The accounts of a subscription entry and their secrets (`providers.md` §
//! Credential vault): the credential pool a provider receives, bound to one
//! entry; the refresh protocol every family follows, with one refresh at a
//! time per account; a pool held in memory; and the account operations a
//! family offers over its pool, and why an account could not be used. The
//! metadata the browser receives is `core`'s.

mod accounts;
mod failure;
mod memory;
mod pool;
mod refresh;

pub use accounts::{
    AccountKit, AccountLabel, Accounts, AccountsCapability, AccountsError, AddAccount,
    LoginError, NewAccount, SubscriptionAccounts,
};
pub use failure::{AuthFailure, AuthReason};
pub use memory::MemoryCredentialPool;
pub use pool::{
    AccountDocument, AccountMeta, CredentialPool, PoolError, Revision, credential_id_for,
    find_by_identity,
};
pub use refresh::{
    AccountError, RefreshGates, RefreshPermit, RenewError, SecretDecodeError, SecretDocument,
    SecretFault, Stored, decode_secret, read_secret, renew,
};

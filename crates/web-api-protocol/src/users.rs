//! Account administration: an administrator lists the accounts, creates one of a
//! role it outranks with a first password the server makes, makes another first
//! password, changes a role or disables an account, sets the size of an account's
//! Cloud disks, signs an account out everywhere, and makes a sign-in code for it.

use std::fmt;

use demi_shared_types::Timestamp;
use garde::Validate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::auth::{Role, UserDto};
use crate::text::EmailAddress;

/// An account as an administrator's list shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListedUser {
    #[serde(flatten)]
    pub user: UserDto,
    /// The latest activity of any of its sessions, to the hour; none while
    /// it has no session, as before its first sign-in.
    pub last_active_at: Option<Timestamp>,
    pub cloud_storage: CloudStorage,
}

/// The size of an account's Cloud disks in whole GB, each from 1 to 1024, which
/// the next start of its Cloud takes; a GB here is 2^30 bytes, as the Cloud's
/// storage shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CloudStorage {
    #[garde(range(min = 1, max = 1024))]
    pub system_gb: u16,
    #[garde(range(min = 1, max = 1024))]
    pub home_gb: u16,
}

impl CloudStorage {
    /// An account's until an administrator changes it, and always the
    /// Claude Code machine's.
    pub const DEFAULT: Self = Self {
        system_gb: 16,
        home_gb: 16,
    };

    pub fn system_bytes(self) -> u64 {
        u64::from(self.system_gb) << 30
    }

    pub fn home_bytes(self) -> u64 {
        u64::from(self.home_gb) << 30
    }
}

/// `GET /users`: every account, in the order they were created.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Users {
    pub users: Vec<ListedUser>,
}

/// The role an administrator gives an account; only setup makes the master.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NewRole {
    Admin,
    User,
}

impl From<NewRole> for Role {
    fn from(role: NewRole) -> Self {
        match role {
            NewRole::Admin => Self::Admin,
            NewRole::User => Self::User,
        }
    }
}

/// `POST /users`: a new account, which signs in without a verification
/// email, with a first password the server makes.
#[derive(Debug, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct CreateUser {
    /// An `EmailAddress` is valid once it is decoded.
    #[garde(skip)]
    pub email: EmailAddress,
    #[garde(skip)]
    pub role: NewRole,
}

/// A first password the server made, shown once to the administrator who
/// asked. `Debug` never shows it.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct FirstPassword(String);

impl FirstPassword {
    pub fn new(password: String) -> Self {
        Self(password)
    }
}

impl fmt::Debug for FirstPassword {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("FirstPassword(..)")
    }
}

/// `{ user, password }`: the 201 answer of `POST /users`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CreatedUser {
    pub user: UserDto,
    pub password: FirstPassword,
}

/// `{ password }`: the answer of `POST /users/:id/password`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NewFirstPassword {
    pub password: FirstPassword,
}

/// `PATCH /users/:id`: one change of an account the caller outranks, its
/// role, whether it is disabled, or the size of its Cloud disks, which an
/// administrator may also change on its own account; the body names
/// exactly one.
#[derive(Debug, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UserChange {
    #[serde(default)]
    #[garde(skip)]
    pub role: Option<NewRole>,
    #[serde(default)]
    #[garde(skip)]
    pub disabled: Option<bool>,
    #[serde(default)]
    #[garde(dive)]
    pub cloud_storage: Option<CloudStorage>,
}

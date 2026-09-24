//! Account administration (`web-api.md` § Account API; `product.md` § User
//! system): an administrator lists the accounts, creates one of a role it
//! outranks, and resets the password of an account it outranks.

use garde::Validate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::auth::{Password, Role, UserDto};
use crate::text::EmailAddress;

/// `GET /users`: every account, in the order they were created.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Users {
    pub users: Vec<UserDto>,
}

/// The role an administrator gives a new account; only setup makes the
/// master.
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
/// email.
#[derive(Debug, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct CreateUser {
    /// An `EmailAddress` is valid once it is decoded.
    #[garde(skip)]
    pub email: EmailAddress,
    #[garde(length(chars, min = 8, max = 1024))]
    pub password: Password,
    #[garde(skip)]
    pub role: NewRole,
}

/// `{ user }`: the 201 answer of `POST /users`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CreatedUser {
    pub user: UserDto,
}

/// `PATCH /users/:id`: the new password of an account the caller outranks.
#[derive(Debug, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct PasswordReset {
    #[garde(length(chars, min = 8, max = 1024))]
    pub password: Password,
}

//! Accounts, password hashing, web sessions, the login lockout and
//! email-change delivery (`backend.md` § Authentication and ownership).

pub(crate) mod email_change;
pub(crate) mod login_limiter;
pub(crate) mod passwords;
pub(crate) mod sessions;

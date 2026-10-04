//! Accounts, password hashing, web sessions, the login lockout and
//! email-change delivery (`backend.md` § Authentication and ownership),
//! each user's preferences (`web-api.md` § User preferences) and each
//! user's subagent settings (`web-api.md` § Subagents). The key email codes
//! are hashed under is given to it; it derives no keys.

pub mod email_change;
pub mod login_limiter;
pub mod passwords;
pub mod sessions;
pub mod settings;
pub mod subagents;

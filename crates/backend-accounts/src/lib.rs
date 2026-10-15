//! Accounts, password hashing, web sessions, the login lockout and
//! email-change delivery (`backend.md` § Authentication and ownership),
//! each user's preferences (`web-api.md` § User preferences) and each
//! user's subagent settings (`web-api.md` § Subagents) and personal
//! instructions (`web-api.md` § Instructions). The key email codes
//! are hashed under is given to it; it derives no keys.

// The text of the mail the backend sends, one catalogue per language
// (`localization.md` § Text from the backend).
rust_i18n::i18n!("locales", fallback = "en");

pub mod email_change;
pub mod instructions;
pub mod login_limiter;
pub mod passkeys;
pub mod passwords;
pub mod sessions;
pub mod settings;
pub mod subagents;
pub mod user_agent;

//! Accounts, password hashing, web sessions, the login lockout and
//! email-change delivery,
//! each user's preferences and each user's subagent settings and personal
//! instructions, and each user's push subscriptions with the sending of push
//! notifications. The key email codes are hashed under, and the VAPID key, are
//! given to it; it derives no keys.
//!
//! The crate knows no conversations, devices or providers.

// The text of the mail the backend sends, one catalogue per language.
rust_i18n::i18n!("locales", fallback = "en");

pub mod email_change;
pub mod instructions;
pub mod login_limiter;
pub mod passkeys;
pub mod passwords;
pub mod push;
pub mod sessions;
pub mod settings;
pub mod subagents;
pub mod user_agent;

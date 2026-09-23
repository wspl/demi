//! Text types whose values are normalized when they arrive.

use std::borrow::Cow;
use std::sync::LazyLock;

use garde::rules::length::utf16::HasUtf16CodeUnits;
use regex::Regex;
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};

/// `text` without the white space around it, as JavaScript's `trim` counts
/// white space: `core`'s one test of blank text decides each character.
fn trim(text: &str) -> &str {
    text.trim_matches(|character: char| demi_core::is_blank(character.encode_utf8(&mut [0; 4])))
}

/// An email address as the product keeps it: trimmed and lowercased, at most
/// 254 UTF-16 code units, and of the form the browser's schema accepts. The
/// type cannot hold another value, so storage lookups and uniqueness see one
/// spelling of each address.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct EmailAddress(String);

/// The most UTF-16 code units an email address has.
pub const EMAIL_MAX: usize = 254;

/// Why a text is not an email address.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EmailError {
    #[error("must be at most {EMAIL_MAX} characters")]
    TooLong,
    #[error("must be an email address")]
    Malformed,
}

impl EmailAddress {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for EmailAddress {
    type Error = EmailError;

    fn try_from(text: String) -> Result<Self, EmailError> {
        let address = trim(&text).to_lowercase();
        if address.encode_utf16().count() > EMAIL_MAX {
            return Err(EmailError::TooLong);
        }
        if !is_email(&address) {
            return Err(EmailError::Malformed);
        }
        Ok(Self(address))
    }
}

/// Zod's `z.email()` rule. Its pattern opens with two lookaheads (no leading
/// `.`, no `..` anywhere), which the `regex` crate does not support, so
/// those two are checked beside the rest of the pattern.
fn is_email(address: &str) -> bool {
    static PATTERN: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^[A-Za-z0-9_'+\-.]*[A-Za-z0-9_+-]@([A-Za-z0-9][A-Za-z0-9\-]*\.)+[A-Za-z]{2,}$")
            .expect("the email pattern compiles")
    });
    !address.starts_with('.') && !address.contains("..") && PATTERN.is_match(address)
}

impl JsonSchema for EmailAddress {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        "EmailAddress".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({ "type": "string", "format": "email", "maxLength": EMAIL_MAX })
    }
}

/// Text whose surrounding white space is removed when it arrives, as a name
/// a user types; the field's garde rule bounds what remains.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "String")]
pub struct Trimmed(String);

impl Trimmed {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl From<String> for Trimmed {
    fn from(text: String) -> Self {
        Self(trim(&text).to_owned())
    }
}

impl HasUtf16CodeUnits for Trimmed {
    fn num_code_units(&self) -> usize {
        self.0.encode_utf16().count()
    }
}

impl JsonSchema for Trimmed {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        "Trimmed".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({ "type": "string" })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn email(text: &str) -> Result<String, EmailError> {
        EmailAddress::try_from(text.to_owned()).map(|address| address.0)
    }

    #[test]
    fn an_email_address_is_trimmed_and_lowercased_before_it_is_checked() {
        assert_eq!(email("  Ana@Example.TEST \n"), Ok("ana@example.test".to_owned()));
        let longest = format!("{}@example.test", "a".repeat(EMAIL_MAX - 13));
        assert_eq!(email(&format!("  {}  ", longest.to_uppercase())), Ok(longest.clone()));
        assert_eq!(email(&format!("a{longest}")), Err(EmailError::TooLong));
    }

    #[test]
    fn an_email_address_has_the_form_the_browser_accepts() {
        for accepted in ["a@b.co", "first.last+tag@sub.example.org", "o'neil_x-y@a-b.example"] {
            assert!(email(accepted).is_ok(), "{accepted}");
        }
        for refused in [
            "invalid",
            ".ana@example.test",
            "ana..b@example.test",
            "ana.@example.test",
            "ana@example",
            "ana@-example.test",
            "ana@example.t",
            "ana@exa_mple.test",
            "an a@example.test",
            "anä@example.test",
            "",
        ] {
            assert_eq!(email(refused), Err(EmailError::Malformed), "{refused}");
        }
    }

    #[test]
    fn text_is_trimmed_of_the_white_space_javascript_trims() {
        let text: Trimmed = serde_json::from_str(r#""﻿  New name \t ""#).unwrap();
        assert_eq!(text.as_str(), "New name");
        assert_eq!(text.num_code_units(), 8);
        // U+0085 is not white space to JavaScript's trim.
        let kept: Trimmed = serde_json::from_str(r#""\u0085name""#).unwrap();
        assert_eq!(kept.as_str(), "\u{85}name");
    }
}

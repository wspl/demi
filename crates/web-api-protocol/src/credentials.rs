//! The user's credentials (`credentials.md`, `web-api.md` § Credentials):
//! what Settings › Credentials reads and sends, the answer to a credential
//! request, and the rules a name, a field and a value follow wherever they
//! arrive, a page's body or an agent's command alike.

use std::collections::{BTreeMap, BTreeSet};

use demi_shared_types::{Nullable, Timestamp};
use garde::Validate;
use jiff::SignedDuration;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::rust::unwrap_or_skip;

use crate::ids::UserCredentialId;
use crate::text::Trimmed;

/// The most characters of a credential's name, after trimming.
pub const NAME_MAX: usize = 80;
/// The most characters of a field's name.
pub const FIELD_NAME_MAX: usize = 32;
/// The most characters of a field's label, after trimming.
pub const LABEL_MAX: usize = 80;
/// The most bytes of a field's value.
pub const VALUE_MAX_BYTES: usize = 64 * 1024;
/// The most fields of a credential.
pub const FIELDS_MAX: usize = 10;

/// What a field's name is made of.
pub const FIELD_NAME_PATTERN: &str = r"^[a-z0-9_-]+$";

/// How a field is meant to be used: a secret is typed hidden and left out of
/// what an agent lists, a visible one is shown as it is. It is advice, not a
/// rule (`credentials.md` § Using a credential).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FieldKind {
    Secret,
    Visible,
}

impl FieldKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Secret => "secret",
            Self::Visible => "visible",
        }
    }
}

/// How long a credential is kept: a time from when it is saved, or until
/// the user removes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub enum Keep {
    #[serde(rename = "1h")]
    Hour,
    #[serde(rename = "1d")]
    Day,
    #[serde(rename = "30d")]
    Month,
    #[serde(rename = "always")]
    Always,
}

impl Keep {
    /// How long from its save it is kept; none for always.
    pub fn duration(self) -> Option<SignedDuration> {
        match self {
            Self::Hour => Some(SignedDuration::from_hours(1)),
            Self::Day => Some(SignedDuration::from_hours(24)),
            Self::Month => Some(SignedDuration::from_hours(24 * 30)),
            Self::Always => None,
        }
    }

    /// The value as a command takes and a body carries it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hour => "1h",
            Self::Day => "1d",
            Self::Month => "30d",
            Self::Always => "always",
        }
    }

    /// The words a command's answer says it with: "kept for 1 hour".
    pub fn phrase(self) -> &'static str {
        match self {
            Self::Hour => "kept for 1 hour",
            Self::Day => "kept for 1 day",
            Self::Month => "kept for 30 days",
            Self::Always => "kept until the user removes it",
        }
    }
}

/// A field as a credential lists it: never its value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase")]
pub struct CredentialField {
    /// 1 to 32 lowercase letters, digits, `_` and `-`, which `read` names.
    #[garde(length(chars, min = 1, max = FIELD_NAME_MAX), pattern(r"^[a-z0-9_-]+$"))]
    pub name: String,
    /// What the card and Settings call it, 1 to 80 characters.
    #[garde(length(chars, min = 1, max = LABEL_MAX))]
    #[schemars(with = "Trimmed")]
    pub label: Trimmed,
    #[garde(skip)]
    pub kind: FieldKind,
}

/// A field with its value, as a new credential carries it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NewField {
    #[garde(length(chars, min = 1, max = FIELD_NAME_MAX), pattern(r"^[a-z0-9_-]+$"))]
    pub name: String,
    #[garde(length(chars, min = 1, max = LABEL_MAX))]
    #[schemars(with = "Trimmed")]
    pub label: Trimmed,
    #[garde(skip)]
    pub kind: FieldKind,
    /// Up to 64 KiB.
    #[garde(length(bytes, max = VALUE_MAX_BYTES))]
    pub value: String,
}

/// A field as a change sends it: without a value, the field keeps the one
/// it has.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChangedField {
    #[garde(length(chars, min = 1, max = FIELD_NAME_MAX), pattern(r"^[a-z0-9_-]+$"))]
    pub name: String,
    #[garde(length(chars, min = 1, max = LABEL_MAX))]
    #[schemars(with = "Trimmed")]
    pub label: Trimmed,
    #[garde(skip)]
    pub kind: FieldKind,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "String")]
    #[garde(inner(length(bytes, max = VALUE_MAX_BYTES)))]
    pub value: Option<String>,
}

/// A field as Settings lists it (`web-api.md` § Credentials): a visible
/// field with its value, a secret one without, which only the reveal reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListedField {
    pub name: String,
    pub label: String,
    pub kind: FieldKind,
    /// A visible field's value; none for a secret one.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "String")]
    pub value: Option<String>,
}

/// A credential's name: trimmed, then 1 to 80 characters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(transparent)]
#[schemars(inline)]
#[garde(transparent)]
pub struct CredentialName(#[garde(length(chars, min = 1, max = NAME_MAX))] pub Trimmed);

impl CredentialName {
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    pub fn into_string(self) -> String {
        self.0.into_string()
    }
}

/// One credential as Settings lists it, its values left out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CredentialDto {
    pub id: UserCredentialId,
    pub name: String,
    /// The reason it was asked for, which the user can edit; empty for none.
    pub note: String,
    pub fields: Vec<ListedField>,
    /// When it is removed; null while it is kept always.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<Timestamp>")]
    pub keep_until: Option<Timestamp>,
    /// When an agent last read it; null before.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<Timestamp>")]
    pub last_used_at: Option<Timestamp>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// `GET /credentials`, and the synchronization channel's `credentials`
/// part: the user's credentials in name order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CredentialList {
    pub credentials: Vec<CredentialDto>,
}

/// What a route that saves a credential answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CredentialAnswer {
    pub credential: CredentialDto,
}

/// `GET /credentials/:id/values`: every field's value by the field's name,
/// for Settings' reveal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CredentialValues {
    pub values: BTreeMap<String, String>,
}

/// `POST /credentials`: a new credential.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NewCredential {
    #[garde(dive)]
    pub name: CredentialName,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "String")]
    #[garde(skip)]
    pub note: Option<String>,
    #[garde(length(min = 1, max = FIELDS_MAX), dive, custom(distinct_new))]
    pub fields: Vec<NewField>,
    #[garde(skip)]
    pub keep: Keep,
}

/// `PATCH /credentials/:id`: the parts it names change, and the others stay.
/// `fields` is the whole list; a field sent without a value keeps the one it
/// has.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CredentialPatch {
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "CredentialName")]
    #[garde(dive)]
    pub name: Option<CredentialName>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "String")]
    #[garde(skip)]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "Vec<ChangedField>")]
    #[garde(dive, custom(changed_fields))]
    pub fields: Option<Vec<ChangedField>>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "Keep")]
    #[garde(skip)]
    pub keep: Option<Keep>,
}

/// The answer to a credential request: every field's value by its name,
/// and how long to keep it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RequestedValues {
    /// Checked against the request's fields where it is answered.
    #[garde(skip)]
    pub values: BTreeMap<String, String>,
    #[garde(skip)]
    pub keep: Keep,
}

/// Each field's name once.
pub fn distinct<'a>(names: impl IntoIterator<Item = &'a str>) -> garde::Result {
    let mut seen = BTreeSet::new();
    for name in names {
        if !seen.insert(name) {
            return Err(garde::Error::new(format!(
                "Two fields are named \u{201c}{name}\u{201d}. Give each field its own name."
            )));
        }
    }
    Ok(())
}

fn distinct_new(fields: &[NewField], _: &()) -> garde::Result {
    distinct(fields.iter().map(|field| field.name.as_str()))
}

/// A patch's whole list: one to ten fields, each named once. Each field's
/// own rules are checked where the list dives into it.
fn changed_fields(fields: &Option<Vec<ChangedField>>, _: &()) -> garde::Result {
    let Some(fields) = fields else {
        return Ok(());
    };
    if fields.is_empty() || fields.len() > FIELDS_MAX {
        return Err(garde::Error::new(format!("holds one to {FIELDS_MAX} fields")));
    }
    distinct(fields.iter().map(|field| field.name.as_str()))
}

/// The words a refused credential body says to the user, who reads them as
/// they are (`web-api.md` § Credentials): one sentence for each rule broken,
/// found by the path of the value that broke it, since garde's own words
/// name the check, not what to do.
pub fn refusal(report: &garde::Report) -> String {
    let mut sentences: Vec<String> = Vec::new();
    for (path, error) in report.iter() {
        let path = path.to_string();
        let sentence = if path == "fields" {
            format!("A credential has 1 to {FIELDS_MAX} fields, each with its own name.")
        } else if path == "name" {
            format!("Enter a name of up to {NAME_MAX} characters.")
        } else if path.ends_with(".name") {
            format!(
                "A field\u{2019}s name has up to {FIELD_NAME_MAX} lowercase letters, digits, \u{201c}_\u{201d} and \u{201c}-\u{201d}."
            )
        } else if path.ends_with(".label") {
            format!("Enter a label of up to {LABEL_MAX} characters for each field.")
        } else if path.ends_with(".value") {
            "A value can be at most 64 KiB.".to_owned()
        } else {
            error.message().to_owned()
        };
        if !sentences.contains(&sentence) {
            sentences.push(sentence);
        }
    }
    sentences.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(name: &str, label: &str) -> CredentialField {
        CredentialField {
            name: name.to_owned(),
            label: Trimmed::from(label.to_owned()),
            kind: FieldKind::Secret,
        }
    }

    #[test]
    fn a_field_name_is_lowercase_letters_digits_underscores_and_hyphens() {
        assert!(field("api_key-2", "API key").validate().is_ok());
        assert!(field("Key", "API key").validate().is_err());
        assert!(field("a key", "API key").validate().is_err());
        assert!(field(&"k".repeat(33), "API key").validate().is_err());
        assert!(field("key", "   ").validate().is_err());
    }
}

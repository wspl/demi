//! Instance settings and each user's preferences (`web-api.md` § User
//! preferences). Preferences hold saved overrides only: whatever is absent
//! uses the browser's defaults.

use demi_command_service::protocol::CommandLocale;
use demi_core::Nullable;
use garde::Validate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::rust::{double_option, unwrap_or_skip};

/// Who configures providers, fixed for the instance's lifetime
/// (`product.md` § Instance mode).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InstanceMode {
    Shared,
    Isolated,
}

serde_plain::derive_display_from_serialize!(InstanceMode);
serde_plain::derive_fromstr_from_deserialize!(InstanceMode);

/// `GET /settings`: the instance's fixed mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Settings {
    pub mode: InstanceMode,
}

/// Whether the page follows the system's light or dark scheme or fixes one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    System,
    Light,
    Dark,
}

/// The page's neutral tone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Tone {
    Ink,
    Warm,
}

/// The page's accent color.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Accent {
    Blue,
    Purple,
    Pink,
    Red,
    Orange,
    Green,
    Teal,
}

/// Appearance overrides. In a patch, each field that is present replaces the
/// saved one.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Appearance {
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "Theme")]
    #[garde(skip)]
    pub theme: Option<Theme>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "Tone")]
    #[garde(skip)]
    pub tone: Option<Tone>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "Accent")]
    #[garde(skip)]
    pub accent: Option<Accent>,
    /// The text size in pixels.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "u8")]
    #[garde(range(min = 12, max = 18))]
    pub font_size: Option<u8>,
}

/// The most UTF-16 code units a shortcut's key sequence has.
pub const SHORTCUT_MAX: usize = 64;

/// Keyboard shortcut overrides: each a key sequence the browser reads.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Shortcuts {
    /// A new conversation.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "String")]
    #[garde(length(utf16, max = SHORTCUT_MAX))]
    pub new: Option<String>,
    /// Showing or hiding the sidebar.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "String")]
    #[garde(length(utf16, max = SHORTCUT_MAX))]
    pub sidebar: Option<String>,
    /// Opening the settings.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "String")]
    #[garde(length(utf16, max = SHORTCUT_MAX))]
    pub settings: Option<String>,
}

/// A change to the shortcut overrides: a key sequence sets one, `null`
/// removes it, and an absent one stays as saved.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShortcutsPatch {
    #[serde(default, skip_serializing_if = "Option::is_none", with = "double_option")]
    #[schemars(with = "Option<String>")]
    #[garde(length(utf16, max = SHORTCUT_MAX))]
    pub new: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "double_option")]
    #[schemars(with = "Option<String>")]
    #[garde(length(utf16, max = SHORTCUT_MAX))]
    pub sidebar: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "double_option")]
    #[schemars(with = "Option<String>")]
    #[garde(length(utf16, max = SHORTCUT_MAX))]
    pub settings: Option<Option<String>>,
}

/// The model a new conversation starts with, as the user last chose it
/// explicitly: existing conversations keep their own selection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LastModel {
    #[garde(length(utf16, min = 1))]
    pub provider_id: String,
    #[garde(length(utf16, min = 1))]
    pub model_id: String,
    /// The thinking effort; null for the model's default.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    #[garde(skip)]
    pub thinking_effort: Option<String>,
    /// The service tier; null for the vendor's default.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    #[garde(skip)]
    pub service_tier_id: Option<String>,
}

/// A user's saved preferences.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Preferences {
    #[garde(dive)]
    pub appearance: Appearance,
    #[garde(dive)]
    pub shortcuts: Shortcuts,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "LastModel")]
    #[garde(dive)]
    pub last_model: Option<LastModel>,
    /// The time zone and languages the browser last reported, which
    /// commands receive in their command context: a zone the backend knows,
    /// in its IANA spelling, and each language once as its canonical tag.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "CommandLocale")]
    #[garde(dive)]
    pub locale: Option<CommandLocale>,
}

/// `PATCH /settings/preferences`: the overrides to change; everything absent
/// stays as saved.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreferencesPatch {
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "Appearance")]
    #[garde(dive)]
    pub appearance: Option<Appearance>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "ShortcutsPatch")]
    #[garde(dive)]
    pub shortcuts: Option<ShortcutsPatch>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "LastModel")]
    #[garde(dive)]
    pub last_model: Option<LastModel>,
    /// A time zone the backend does not know or a malformed language tag is
    /// refused.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "CommandLocale")]
    #[garde(dive)]
    pub locale: Option<CommandLocale>,
}

/// `{ preferences }`: the answer of `GET` and `PATCH /settings/preferences`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UserPreferences {
    pub preferences: Preferences,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_patch_schema_is_strict_optional_throughout_and_bounded() {
        let schema = serde_json::to_value(schemars::schema_for!(PreferencesPatch)).unwrap();
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(schema["required"], serde_json::json!(null));
        let definitions = &schema["$defs"];
        assert_eq!(definitions["Appearance"]["properties"]["fontSize"]["maximum"], 18);
        assert_eq!(definitions["ShortcutsPatch"]["properties"]["new"]["type"], serde_json::json!(["string", "null"]));
        assert_eq!(definitions["CommandLocale"]["properties"]["languages"]["maxItems"], 16);
        assert_eq!(definitions["CommandLocale"]["additionalProperties"], false);
    }
}

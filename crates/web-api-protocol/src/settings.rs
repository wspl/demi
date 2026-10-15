//! Instance settings and each user's preferences (`web-api.md` § User
//! preferences). Preferences hold saved overrides only: whatever is absent
//! uses the web app's defaults.

use std::collections::BTreeMap;

use demi_command_protocol::{ColorScheme, CommandLocale};
use demi_shared_types::{Nullable, is_context_limit};
use garde::Validate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::rust::{double_option, unwrap_or_skip};

use crate::conversations::ModelSettings;
use crate::ids::{DeviceId, ProviderId};

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
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "Theme")]
    #[garde(skip)]
    pub theme: Option<Theme>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "Tone")]
    #[garde(skip)]
    pub tone: Option<Tone>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "Accent")]
    #[garde(skip)]
    pub accent: Option<Accent>,
    /// The text size in pixels.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "u8")]
    #[garde(range(min = 12, max = 18))]
    pub font_size: Option<u8>,
}

/// The most characters (Unicode scalar values) a shortcut's key sequence
/// has.
pub const SHORTCUT_MAX: usize = 64;

/// Keyboard shortcut overrides: each a key sequence the web app reads.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Shortcuts {
    /// A new conversation.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "String")]
    #[garde(length(chars, max = SHORTCUT_MAX))]
    pub new: Option<String>,
    /// Showing or hiding the sidebar.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "String")]
    #[garde(length(chars, max = SHORTCUT_MAX))]
    pub sidebar: Option<String>,
    /// Opening the settings.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "String")]
    #[garde(length(chars, max = SHORTCUT_MAX))]
    pub settings: Option<String>,
    /// Opening the search window.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "String")]
    #[garde(length(chars, max = SHORTCUT_MAX))]
    pub search: Option<String>,
}

/// A change to the shortcut overrides: a key sequence sets one, `null`
/// removes it, and an absent one stays as saved.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShortcutsPatch {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    #[schemars(with = "Option<String>")]
    #[garde(length(chars, max = SHORTCUT_MAX))]
    pub new: Option<Option<String>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    #[schemars(with = "Option<String>")]
    #[garde(length(chars, max = SHORTCUT_MAX))]
    pub sidebar: Option<Option<String>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    #[schemars(with = "Option<String>")]
    #[garde(length(chars, max = SHORTCUT_MAX))]
    pub settings: Option<Option<String>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    #[schemars(with = "Option<String>")]
    #[garde(length(chars, max = SHORTCUT_MAX))]
    pub search: Option<Option<String>>,
}

/// Where a new project lives: the user's Cloud or one of their devices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProjectHostKind {
    Cloud,
    Device,
}

/// Where New project starts (`product.md` § Conversations and projects): the
/// kind the user last chose and the device last chosen in its device menu,
/// kept while the Cloud is chosen. A device the user no longer has stays
/// here; the dialog chooses no device for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectHostChoice {
    #[garde(skip)]
    pub kind: ProjectHostKind,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "DeviceId")]
    #[garde(skip)]
    pub device_id: Option<DeviceId>,
}

/// What a message the user sends while the agent works does (`product.md` §
/// Steer or queue): steer the running turn or wait in the queue. Absent
/// means steer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SendWhileRunning {
    Steer,
    Queue,
}

/// A language Demi speaks (`localization.md`): its text, the page's and
/// the backend's, is written in each.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Language {
    #[serde(rename = "en")]
    English,
    #[serde(rename = "zh-Hans")]
    SimplifiedChinese,
    #[serde(rename = "zh-Hant")]
    TraditionalChinese,
    #[serde(rename = "ja")]
    Japanese,
    #[serde(rename = "ko")]
    Korean,
}

serde_plain::derive_display_from_serialize!(Language);

impl Language {
    /// The first of `tags`, BCP 47 language tags in the order of
    /// preference, that Demi speaks; English when none is
    /// (`localization.md` § Choosing the language).
    pub fn first_spoken<'a>(tags: impl IntoIterator<Item = &'a str>) -> Self {
        tags.into_iter()
            .find_map(Self::of_tag)
            .unwrap_or(Self::English)
    }

    /// The language Demi speaks for `tag`, if any: Traditional Chinese
    /// for `zh-Hant` and the Chinese of Taiwan, Hong Kong and Macao, which
    /// write it, and Simplified Chinese for any other `zh`. A script the tag
    /// names wins over its region: `zh-Hans-HK` is Simplified.
    fn of_tag(tag: &str) -> Option<Self> {
        let mut subtags = tag.split(['-', '_']);
        let primary = subtags.next().unwrap_or_default().to_ascii_lowercase();
        match primary.as_str() {
            "en" => Some(Self::English),
            "ja" => Some(Self::Japanese),
            "ko" => Some(Self::Korean),
            "zh" => {
                let mut traditional_region = false;
                for subtag in subtags {
                    if subtag.eq_ignore_ascii_case("hant") {
                        return Some(Self::TraditionalChinese);
                    }
                    if subtag.eq_ignore_ascii_case("hans") {
                        return Some(Self::SimplifiedChinese);
                    }
                    traditional_region |= ["tw", "hk", "mo"]
                        .iter()
                        .any(|region| subtag.eq_ignore_ascii_case(region));
                }
                Some(if traditional_region {
                    Self::TraditionalChinese
                } else {
                    Self::SimplifiedChinese
                })
            }
            _ => None,
        }
    }
}

/// The language the user chose in Settings › General: one Demi speaks, or
/// the system's, which follows the browser's languages. Absent means
/// `system`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub enum LanguageChoice {
    #[serde(rename = "system")]
    System,
    #[serde(rename = "en")]
    English,
    #[serde(rename = "zh-Hans")]
    SimplifiedChinese,
    #[serde(rename = "zh-Hant")]
    TraditionalChinese,
    #[serde(rename = "ja")]
    Japanese,
    #[serde(rename = "ko")]
    Korean,
}

impl LanguageChoice {
    /// The language the choice speaks, the first of `browser`'s that Demi
    /// speaks for `system`.
    pub fn resolve<'a>(self, browser: impl IntoIterator<Item = &'a str>) -> Language {
        match self {
            Self::System => Language::first_spoken(browser),
            Self::English => Language::English,
            Self::SimplifiedChinese => Language::SimplifiedChinese,
            Self::TraditionalChinese => Language::TraditionalChinese,
            Self::Japanese => Language::Japanese,
            Self::Korean => Language::Korean,
        }
    }
}

/// A user's saved preferences.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Preferences {
    #[garde(dive)]
    pub appearance: Appearance,
    #[garde(dive)]
    pub shortcuts: Shortcuts,
    /// The model settings a new conversation starts with, as the user last
    /// chose them; existing conversations keep their own.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "ModelSettings")]
    #[garde(dive)]
    pub last_model: Option<ModelSettings>,
    /// Where New project starts, as the user last chose it.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "ProjectHostChoice")]
    #[garde(dive)]
    pub last_project_host: Option<ProjectHostChoice>,
    /// The language of the user's pages and of the mail the backend sends
    /// them (`localization.md` § Choosing the language).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "LanguageChoice")]
    #[garde(skip)]
    pub language: Option<LanguageChoice>,
    /// The time zone and languages the user's browser last reported, which
    /// commands receive in their command context: a zone the backend knows,
    /// in its IANA spelling, and each language once as its canonical tag.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "CommandLocale")]
    #[garde(dive)]
    pub locale: Option<CommandLocale>,
    /// The color scheme the user's page last reported showing, which the
    /// conversation browser starts with.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "ColorScheme")]
    #[garde(skip)]
    pub color_scheme: Option<ColorScheme>,
    /// What Enter does with a message while the agent works.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "SendWhileRunning")]
    #[garde(skip)]
    pub send_while_running: Option<SendWhileRunning>,
    /// Whether a turn that failed with a known time to try again, such as a
    /// usage limit that resets at 09:00, resumes by itself at that time
    /// (`failures-and-recovery.md` § The unfinished turn). Absent means no.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "bool")]
    #[garde(skip)]
    pub resume_at_retry: Option<bool>,
    /// Whether the user went on past the dialog about using several
    /// subscription accounts (`providers.md` § Account rotation), which then
    /// never shows again. Absent means no.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "bool")]
    #[garde(skip)]
    pub multi_account_warned: Option<bool>,
    /// The limit the user set on the context window of each model, in
    /// tokens, by provider entry id and then model id (`models.md` § Context
    /// limit); a model it does not name uses its full window.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[garde(custom(stored_limits))]
    pub context_limits: ContextLimits,
    /// The provider entries the user turned off in Settings › Providers,
    /// which every model menu and `demi agent models` leave out without the
    /// entry being deleted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[garde(skip)]
    pub hidden_providers: Vec<ProviderId>,
    /// The models the user turned off in Settings › Providers, each of one
    /// entry, which the menus leave out as they do a hidden entry.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[garde(dive)]
    pub hidden_models: Vec<HiddenModel>,
}

impl Preferences {
    /// Whether the user turned the entry `provider` off.
    pub fn hides_provider(&self, provider: &ProviderId) -> bool {
        self.hidden_providers.contains(provider)
    }

    /// Whether the user turned the model `model` of the entry `provider`
    /// off.
    pub fn hides_model(&self, provider: &ProviderId, model: &str) -> bool {
        self.hidden_models
            .iter()
            .any(|hidden| hidden.provider_id == *provider && hidden.model_id == model)
    }
}

/// A model the user turned off, of one provider entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HiddenModel {
    #[garde(skip)]
    pub provider_id: ProviderId,
    #[garde(length(chars, min = 1))]
    pub model_id: String,
}

/// The context limits of a user, by provider entry id and then model id.
pub type ContextLimits = BTreeMap<ProviderId, BTreeMap<String, u32>>;

/// Every stored limit is one a window offers.
fn stored_limits(limits: &ContextLimits, (): &()) -> garde::Result {
    let offered = limits
        .values()
        .flat_map(BTreeMap::values)
        .all(|tokens| is_context_limit(*tokens));
    if offered {
        Ok(())
    } else {
        Err(garde::Error::new("a context limit no window offers"))
    }
}

/// One model's context limit, as a preferences patch changes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContextLimitChange {
    #[garde(skip)]
    pub provider_id: ProviderId,
    #[garde(length(chars, min = 1))]
    pub model_id: String,
    /// 500000, 300000 or 200000 tokens; null removes the limit, so the model
    /// uses its full window.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<u32>")]
    #[garde(custom(offered_limit))]
    pub tokens: Option<u32>,
}

/// A limit to store is one a window offers.
fn offered_limit(tokens: &Option<u32>, (): &()) -> garde::Result {
    match tokens {
        Some(tokens) if !is_context_limit(*tokens) => Err(garde::Error::new(format!(
            "{tokens} is not a context limit: 500000, 300000 or 200000"
        ))),
        _ => Ok(()),
    }
}

/// `PATCH /settings/preferences`: the overrides to change; everything absent
/// stays as saved.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreferencesPatch {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "Appearance")]
    #[garde(dive)]
    pub appearance: Option<Appearance>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "ShortcutsPatch")]
    #[garde(dive)]
    pub shortcuts: Option<ShortcutsPatch>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "ModelSettings")]
    #[garde(dive)]
    pub last_model: Option<ModelSettings>,
    /// Replaces the saved choice whole.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "ProjectHostChoice")]
    #[garde(dive)]
    pub last_project_host: Option<ProjectHostChoice>,
    /// The language of the user's pages and of the mail the backend sends
    /// them (`localization.md` § Choosing the language).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "LanguageChoice")]
    #[garde(skip)]
    pub language: Option<LanguageChoice>,
    /// A time zone the backend does not know or a malformed language tag is
    /// refused.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "CommandLocale")]
    #[garde(dive)]
    pub locale: Option<CommandLocale>,
    /// The color scheme the user's page shows.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "ColorScheme")]
    #[garde(skip)]
    pub color_scheme: Option<ColorScheme>,
    /// What Enter does with a message while the agent works.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "SendWhileRunning")]
    #[garde(skip)]
    pub send_while_running: Option<SendWhileRunning>,
    /// Whether a turn that failed with a known time to try again, such as a
    /// usage limit that resets at 09:00, resumes by itself at that time
    /// (`failures-and-recovery.md` § The unfinished turn). Absent means no.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "bool")]
    #[garde(skip)]
    pub resume_at_retry: Option<bool>,
    /// Whether the user went on past the dialog about using several
    /// subscription accounts (`providers.md` § Account rotation), which then
    /// never shows again. Absent means no.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "bool")]
    #[garde(skip)]
    pub multi_account_warned: Option<bool>,
    /// One model's context limit; the other models keep theirs.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "ContextLimitChange")]
    #[garde(dive)]
    pub context_limit: Option<ContextLimitChange>,
    /// Replaces the list of the entries the user turned off.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "Vec<ProviderId>")]
    #[garde(skip)]
    pub hidden_providers: Option<Vec<ProviderId>>,
    /// Replaces the list of the models the user turned off.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "Vec<HiddenModel>")]
    #[garde(dive)]
    pub hidden_models: Option<Vec<HiddenModel>>,
}

/// `{ preferences }`: the answer of `GET` and `PATCH /settings/preferences`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UserPreferences {
    pub preferences: Preferences,
}

#[cfg(test)]
mod tests {
    use super::*;

    // The browser's languages, as `navigator.languages` or a stored locale
    // lists them, and the language System speaks for them.
    #[test]
    fn system_speaks_the_first_browser_language_demi_speaks() {
        use Language::*;
        for (browser, spoken) in [
            (&["zh-TW"][..], TraditionalChinese),
            (&["zh-HK"], TraditionalChinese),
            (&["zh-MO"], TraditionalChinese),
            (&["zh-Hant"], TraditionalChinese),
            (&["zh-Hans-HK"], SimplifiedChinese),
            (&["zh-CN"], SimplifiedChinese),
            (&["zh"], SimplifiedChinese),
            (&["en-GB", "zh-CN"], English),
            (&["fr", "ja-JP", "en"], Japanese),
            (&["fr", "ko-KR"], Korean),
            (&["fr"], English),
            (&[], English),
        ] {
            assert_eq!(
                LanguageChoice::System.resolve(browser.iter().copied()),
                spoken,
                "{browser:?}"
            );
        }
        assert_eq!(
            LanguageChoice::Korean.resolve(["zh-TW"]),
            Korean,
            "a chosen language is spoken whatever the browser's"
        );
    }

    #[test]
    fn the_patch_schema_is_strict_optional_throughout_and_bounded() {
        let schema = serde_json::to_value(schemars::schema_for!(PreferencesPatch)).unwrap();
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(schema["required"], serde_json::json!(null));
        let definitions = &schema["$defs"];
        assert_eq!(
            definitions["Appearance"]["properties"]["fontSize"]["maximum"],
            18
        );
        assert_eq!(
            definitions["ShortcutsPatch"]["properties"]["new"]["type"],
            serde_json::json!(["string", "null"])
        );
        assert_eq!(
            definitions["CommandLocale"]["properties"]["languages"]["maxItems"],
            16
        );
        assert_eq!(definitions["CommandLocale"]["additionalProperties"], false);
    }
}

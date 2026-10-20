//! Provider entries, their accounts and logins, and the account-wide model
//! catalog. No body the backend answers carries key or token material.

use demi_shared_types::{
    AccountInfo, AudioKind, AudioShape, AuthState, FileExtension, ImageShape, LoginStage, MediaKind, ModelSelection, VideoShape,
    Nullable, ProviderModel, QuotaSnapshot, RequestId, RuntimeState, Timestamp, WireApi,
};
use garde::Validate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::rust::{double_option, unwrap_or_skip};

use crate::ids::{CredentialId, LoginId, ProviderId};
use crate::text::{EndpointUrl, Trimmed};

/// The most characters (Unicode scalar values) an entry's label has, after
/// trimming.
pub const LABEL_MAX: usize = 80;

/// The most characters (Unicode scalar values) a pasted sign-in code has,
/// after trimming.
pub const CODE_MAX: usize = 4096;

/// How an entry authenticates (`crates/backend-providers/src/llm/families.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CredentialKind {
    /// A key the user types in.
    ApiKey,
    /// Accounts from a sign-in.
    Subscription,
}

serde_plain::derive_display_from_serialize!(CredentialKind);
serde_plain::derive_fromstr_from_deserialize!(CredentialKind);

pub use demi_shared_types::Rotation;

/// What a configured model makes (`crates/backend-user-shard/src/provider_settings.rs`):
/// text, the model menu's, unless its user added it as an image, video or
/// audio model of a compatible entry with a shape for that kind.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ModelKind {
    #[default]
    Text,
    Image,
    Video,
    Audio,
}

impl ModelKind {
    /// The kind of media a model of this kind makes; none for text.
    pub fn media(self) -> Option<MediaKind> {
        match self {
            Self::Text => None,
            Self::Image => Some(MediaKind::Image),
            Self::Video => Some(MediaKind::Video),
            Self::Audio => Some(MediaKind::Audio),
        }
    }
}

serde_plain::derive_display_from_serialize!(ModelKind);
serde_plain::derive_fromstr_from_deserialize!(ModelKind);

/// One model of an entry's configured list, which states its facts directly
/// (`crates/shared-types/src/catalog.rs`): its first thinking effort is its
/// default, its Fast tier, when it names one, is its only service tier, and
/// it is taken to call tools. A model that is not text uses only its ID
/// and name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfiguredModel {
    /// Text when absent, as every model stored before kinds was.
    #[serde(default)]
    #[garde(skip)]
    pub kind: ModelKind,
    #[garde(length(chars, min = 1, max = 256))]
    pub id: Trimmed,
    #[garde(length(chars, min = 1, max = 256))]
    pub display_name: Trimmed,
    /// Tokens; null while the window is unknown, as a model added by Custom
    /// has until the user sets one.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<u32>")]
    #[garde(inner(range(min = 1)))]
    pub context_window: Option<u32>,
    /// Null for no model-specific limit; never beyond a known context
    /// window.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<u32>")]
    #[garde(range(min = 1), custom(within(self.context_window)))]
    pub output_limit: Option<u32>,
    /// The vendor's effort levels; empty for a model without thinking.
    #[garde(length(max = 32), inner(length(chars, min = 1, max = 64)))]
    pub thinking_efforts: Vec<String>,
    /// The types the model reads natively: `[]` for none, null when unknown.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<Vec<FileExtension>>")]
    #[garde(length(max = 64))]
    pub accepted_extensions: Option<Vec<FileExtension>>,
    /// The service tier the model's Fast is; null for none.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    #[garde(length(chars, min = 1, max = 64))]
    pub fast_tier: Option<String>,
}

/// An output limit is at most the context window, when the window is known.
fn within(context_window: Option<u32>) -> impl FnOnce(&Option<u32>, &()) -> garde::Result {
    move |output_limit, ()| match (output_limit, context_window) {
        (Some(limit), Some(window)) if *limit > window => Err(garde::Error::new(
            "the output limit exceeds the context window",
        )),
        _ => Ok(()),
    }
}

/// An entry's complete manual model list: at most 1000 models. Their ids
/// are distinct in a list that is kept ([`ConfiguredModels::repeated_id`]):
/// a request that repeats one is refused as `duplicate_model`, not as an
/// invalid body. An empty list leaves the entry incomplete until a model is
/// added (`crates/backend-user-shard/src/provider_settings.rs`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(transparent)]
#[garde(transparent)]
pub struct ConfiguredModels(#[garde(length(max = 1000), dive)] pub Vec<ConfiguredModel>);

impl ConfiguredModels {
    /// The first id that a model of the list shares with an earlier one.
    pub fn repeated_id(&self) -> Option<&str> {
        let mut seen = std::collections::HashSet::new();
        self.0
            .iter()
            .map(|model| model.id.as_str())
            .find(|id| !seen.insert(*id))
    }
}

/// `POST /providers`: a vendor entry from the vendor list, whose family,
/// wire and endpoint the vendor supplies; a vendor entry of a family's own
/// vendor, such as Anthropic's; or a compatible entry, Anthropic Messages
/// unless it names another protocol. Everything but the kind may come later:
/// the entry is incomplete until it has what it needs, and its label is the
/// vendor's or the family's name, or *Compatible API*, with the first free
/// *(2)*, *(3)* when the scope has an entry of that name.
#[derive(Debug, Deserialize, JsonSchema, Validate)]
#[serde(
    tag = "source",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum CreateProvider {
    Vendor {
        #[garde(length(min = 1))]
        vendor_id: String,
        #[serde(default, with = "unwrap_or_skip")]
        #[schemars(with = "Trimmed")]
        #[garde(length(chars, min = 1, max = LABEL_MAX))]
        label: Option<Trimmed>,
        #[serde(default, with = "unwrap_or_skip")]
        #[schemars(with = "String")]
        #[garde(length(min = 1))]
        api_key: Option<String>,
        /// Replaces the vendor's live model list.
        #[serde(default, with = "unwrap_or_skip")]
        #[schemars(with = "ConfiguredModels")]
        #[garde(dive)]
        models: Option<ConfiguredModels>,
    },
    Family {
        #[garde(length(min = 1))]
        provider_type: String,
        /// For the `openai` family only; its default is Responses.
        #[serde(default, with = "unwrap_or_skip")]
        #[schemars(with = "WireApi")]
        #[garde(skip)]
        wire_api: Option<WireApi>,
        #[serde(default, with = "unwrap_or_skip")]
        #[schemars(with = "Trimmed")]
        #[garde(length(chars, min = 1, max = LABEL_MAX))]
        label: Option<Trimmed>,
        #[serde(default, with = "unwrap_or_skip")]
        #[schemars(with = "String")]
        #[garde(length(min = 1))]
        api_key: Option<String>,
        #[serde(default, with = "unwrap_or_skip")]
        #[schemars(with = "ConfiguredModels")]
        #[garde(dive)]
        models: Option<ConfiguredModels>,
    },
    Compatible {
        /// The protocol's family; `anthropic` without it.
        #[serde(default, with = "unwrap_or_skip")]
        #[schemars(with = "String")]
        #[garde(length(min = 1))]
        provider_type: Option<String>,
        /// For the `openai` family only; its default is Responses.
        #[serde(default, with = "unwrap_or_skip")]
        #[schemars(with = "WireApi")]
        #[garde(skip)]
        wire_api: Option<WireApi>,
        #[serde(default, with = "unwrap_or_skip")]
        #[schemars(with = "Trimmed")]
        #[garde(length(chars, min = 1, max = LABEL_MAX))]
        label: Option<Trimmed>,
        /// Optional for good: a server of the user's own may take none.
        #[serde(default, with = "unwrap_or_skip")]
        #[schemars(with = "String")]
        #[garde(length(min = 1))]
        api_key: Option<String>,
        #[serde(default, with = "unwrap_or_skip")]
        #[schemars(with = "EndpointUrl")]
        #[garde(skip)]
        base_url: Option<EndpointUrl>,
        #[serde(default, with = "unwrap_or_skip")]
        #[schemars(with = "ConfiguredModels")]
        #[garde(dive)]
        models: Option<ConfiguredModels>,
    },
}

/// `PATCH /providers/:id`: a new label for any entry; for an API-key entry
/// also a new key, where `null` removes it, and a new model list, where
/// `null` returns to the vendor's catalog; and, for a compatible entry only,
/// a new endpoint, where `null` removes it, and a new protocol: a
/// `providerType` with the `wireApi` it speaks, or a `wireApi` alone for the
/// entry's own family (`crates/provider-common/src/endpoint.rs`).
#[derive(Debug, Default, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderPatch {
    #[serde(default, with = "unwrap_or_skip")]
    #[schemars(with = "Trimmed")]
    #[garde(length(chars, min = 1, max = LABEL_MAX))]
    pub label: Option<Trimmed>,
    #[serde(default, with = "double_option")]
    #[schemars(with = "Option<String>")]
    #[garde(inner(inner(length(min = 1))))]
    pub api_key: Option<Option<String>>,
    #[serde(default, with = "double_option")]
    #[schemars(with = "Option<EndpointUrl>")]
    #[garde(skip)]
    pub base_url: Option<Option<EndpointUrl>>,
    /// The compatible entry's new family; its wire is then `wire_api`, or
    /// the family's default without it.
    #[serde(default, with = "unwrap_or_skip")]
    #[schemars(with = "String")]
    #[garde(length(min = 1))]
    pub provider_type: Option<String>,
    #[serde(default, with = "unwrap_or_skip")]
    #[schemars(with = "WireApi")]
    #[garde(skip)]
    pub wire_api: Option<WireApi>,
    #[serde(default, with = "double_option")]
    #[schemars(with = "Option<ConfiguredModels>")]
    #[garde(dive)]
    pub models: Option<Option<ConfiguredModels>>,
    /// A subscription entry's rotation; one with fewer than two accounts
    /// keeps `manual`.
    #[serde(default, with = "unwrap_or_skip")]
    #[schemars(with = "Rotation")]
    #[garde(skip)]
    pub rotation: Option<Rotation>,
    /// The entry's media shapes and switches (`media.md` § Which entries
    /// make which media).
    #[serde(default)]
    #[garde(skip)]
    pub media: Option<MediaPatch>,
}

/// A change of an entry's media, by kind.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MediaPatch {
    #[serde(default)]
    pub image: Option<ImageMediaPatch>,
    #[serde(default)]
    pub video: Option<VideoMediaPatch>,
    #[serde(default)]
    pub audio: Option<AudioMediaPatch>,
}

/// A change of how an entry makes images: `shape`, a compatible entry's
/// only, null for none; `enabled`, its Image Generation switch, for an
/// entry that makes images.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImageMediaPatch {
    #[serde(default, with = "double_option")]
    #[schemars(with = "Option<Nullable<ImageShape>>")]
    pub shape: Option<Option<ImageShape>>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

/// A change of how an entry makes videos: `shape`, a compatible entry's
/// only, null for none; `enabled`, its Video Generation switch, for an
/// entry that makes videos.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VideoMediaPatch {
    #[serde(default, with = "double_option")]
    #[schemars(with = "Option<Nullable<VideoShape>>")]
    pub shape: Option<Option<VideoShape>>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

/// A change of how an entry makes audio: `shape`, a compatible entry's
/// only, null for none; `enabled`, its Audio Generation switch, for an
/// entry that makes audio.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AudioMediaPatch {
    #[serde(default, with = "double_option")]
    #[schemars(with = "Option<Nullable<AudioShape>>")]
    pub shape: Option<Option<AudioShape>>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

impl MediaPatch {
    /// The switch the patch sets for `kind`.
    pub fn enabled(&self, kind: MediaKind) -> Option<bool> {
        match kind {
            MediaKind::Image => self.image.as_ref().and_then(|image| image.enabled),
            MediaKind::Video => self.video.as_ref().and_then(|video| video.enabled),
            MediaKind::Audio => self.audio.as_ref().and_then(|audio| audio.enabled),
        }
    }
}

/// How an entry makes each kind of media.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct EntryMedia {
    pub image: ImageMedia,
    pub video: VideoMedia,
    pub audio: AudioMedia,
    /// When the media catalog that describes its models was published: a
    /// newer catalog changes it, so the page reads the entry's media models
    /// again.
    pub catalog: Timestamp,
}

/// How an entry makes images: its shape, null when it makes none, and its
/// Image Generation switch, on from the start.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImageMedia {
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<ImageShape>")]
    pub shape: Option<ImageShape>,
    pub enabled: bool,
}

/// How an entry makes videos: its shape, null when it makes none, and its
/// Video Generation switch, on from the start.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct VideoMedia {
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<VideoShape>")]
    pub shape: Option<VideoShape>,
    pub enabled: bool,
}

/// How an entry makes audio: its shape, null when it makes none, and its
/// Audio Generation switch, on from the start.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AudioMedia {
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<AudioShape>")]
    pub shape: Option<AudioShape>,
    pub enabled: bool,
}

/// `PUT /providers/order`: every entry of the caller's scope once, in the
/// order Settings › Providers shows.
#[derive(Debug, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderOrder {
    #[garde(skip)]
    pub provider_ids: Vec<ProviderId>,
}

/// An entry as the web app sees it: its family, label, kind, endpoint, vendor
/// and model list, never its key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProviderDto {
    pub id: ProviderId,
    pub kind: CredentialKind,
    /// The entry's family, such as `openai` or `codex`.
    pub provider_type: String,
    pub label: String,
    /// The wire of an `openai` entry that names one; null otherwise.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<WireApi>")]
    pub wire_api: Option<WireApi>,
    /// The vendor of a vendor entry, its family's own included; null for a
    /// compatible entry and a subscription.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    pub vendor_id: Option<String>,
    /// A compatible entry (`crates/provider-common/src/endpoint.rs`): its endpoint is its
    /// own and its models are the ones the user adds. Any other API-key
    /// entry is a vendor entry; false for a subscription.
    pub compatible: bool,
    /// The base URL an API-key entry's requests go to, whichever kind it
    /// is: the configured one, else, for a vendor entry, the family's
    /// default; null for a compatible entry not given one yet and for a
    /// subscription.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<EndpointUrl>")]
    pub endpoint: Option<EndpointUrl>,
    /// Whether an API-key entry has its key saved; false for a
    /// subscription. The key itself is never sent.
    pub has_api_key: bool,
    /// The configured model list; null for the entry's live catalog.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<ConfiguredModels>")]
    pub models: Option<ConfiguredModels>,
    /// Which account each agent uses; `manual` for an API-key entry.
    pub rotation: Rotation,
    /// How it makes each kind of media.
    pub media: EntryMedia,
    pub created_at: Timestamp,
}

/// `{ provider }`: the answer of a create and an edit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProviderAnswer {
    pub provider: ProviderDto,
}

/// `GET /providers`: the entries of the caller's scope, in the order
/// Settings › Providers shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Providers {
    pub providers: Vec<ProviderDto>,
}

/// How the product learns an account's quota.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum QuotaCapability {
    /// The family reports no quota.
    None {},
    /// The family reports quota, from responses and, when `probe` names its
    /// cost, from a probe of its usage endpoint.
    Supported {
        #[serde(deserialize_with = "Option::deserialize")]
        #[schemars(with = "Nullable<ProbeCost>")]
        probe: Option<ProbeCost>,
    },
}

/// What a quota probe costs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProbeCost {
    /// It reads a usage endpoint.
    Free,
    /// It would spend an inference request, which the backend never runs.
    Inference,
}

/// An account with the quota snapshot kept for it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AccountDto {
    #[serde(flatten)]
    pub account: AccountInfo,
    /// The account's last real snapshot; null before any.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<QuotaSnapshot>")]
    pub quota: Option<QuotaSnapshot>,
}

/// What `GET /providers/:id/status` answers, read when it answers and never
/// stored: the provider's health, its accounts and their quota. For a user
/// who only infers with the entry, `accounts` is empty, `active` and `quota`
/// are null, and an authenticated `auth` names no account.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProviderDetails {
    pub auth: AuthState,
    pub runtime: RuntimeState,
    pub accounts: Vec<AccountDto>,
    /// The account the entry infers with.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<CredentialId>")]
    pub active: Option<CredentialId>,
    /// The active account's snapshot.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<QuotaSnapshot>")]
    pub quota: Option<QuotaSnapshot>,
    pub quota_capability: QuotaCapability,
    /// The command package that installs the CLI the provider's requests
    /// start on the Claude Code machine, such as `demi.claude-code`; null
    /// when its requests are HTTP.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    pub cli_package: Option<String>,
}

/// An entry in the product state: the entry and what the backend could read
/// of its provider.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProviderState {
    #[serde(flatten)]
    pub provider: ProviderDto,
    pub details: ProviderReading,
}

/// What the backend read of an entry's provider when it answered.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProviderReading {
    Read(Box<ProviderDetails>),
    /// The provider could not be built or read; the other entries are
    /// unaffected.
    Failed {
        message: String,
    },
}

/// `POST /providers/:id/quota`: the account to probe; the active one
/// without it.
#[derive(Debug, Default, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QuotaRequest {
    #[serde(default, with = "unwrap_or_skip")]
    #[schemars(with = "CredentialId")]
    #[garde(skip)]
    pub credential_id: Option<CredentialId>,
}

/// `{ quota }`: the account's snapshot after the probe, or the kept one of a
/// family that cannot probe; null when there is none.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct QuotaAnswer {
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<QuotaSnapshot>")]
    pub quota: Option<QuotaSnapshot>,
}

/// `POST /providers/:id/test`: one real request to the model, with the
/// account the caller names or the active one.
#[derive(Debug, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TestRequest {
    /// The text model the test sends its message to; absent for an entry
    /// of media alone, whose key is checked through its vendor's list of
    /// models (`crates/backend/src/families.rs`).
    #[serde(default, with = "unwrap_or_skip")]
    #[schemars(with = "String")]
    #[garde(inner(length(min = 1)))]
    pub model_id: Option<String>,
    #[serde(default, with = "unwrap_or_skip")]
    #[schemars(with = "CredentialId")]
    #[garde(skip)]
    pub credential_id: Option<CredentialId>,
}

/// What a test found. A test that ran and failed is a result, with the
/// provider's own reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum TestResult {
    /// The model answered; `model` is its display name.
    Passed { model: String },
    Failed {
        message: String,
        /// The model's display name, when the catalog lists it.
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        #[schemars(with = "String")]
        model: Option<String>,
        /// For a request to a port of the entry's device that the device
        /// has not exposed, that port, which the page offers to expose
        /// with Expose Port before testing again (`crates/backend-user-shard/src/shard/exposed_ports.rs`).
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        #[schemars(with = "demi_shared_types::NotExposedPort")]
        not_exposed: Option<demi_shared_types::NotExposedPort>,
    },
}

/// `GET /providers/catalog`: what the page can add.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct VendorCatalog {
    /// Each subscription family, with whether the scope holds its entry.
    pub subscriptions: Vec<SubscriptionFamily>,
    /// The models.dev vendors a family speaks to, by name.
    pub vendors: Vec<Vendor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SubscriptionFamily {
    pub provider_type: String,
    pub configured: bool,
}

/// A vendor the page can add an entry from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Vendor {
    pub id: String,
    pub name: String,
    /// The family that speaks the vendor's protocol.
    pub provider_type: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "WireApi")]
    pub wire_api: Option<WireApi>,
    /// The endpoint an entry starts with.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    pub base_url: Option<String>,
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    pub doc: Option<String>,
}

/// `GET /providers/:id/accounts`: the entry's accounts and the active one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Accounts {
    pub accounts: Vec<AccountInfo>,
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<CredentialId>")]
    pub active: Option<CredentialId>,
}

/// `PUT /providers/:id/accounts/active`.
#[derive(Debug, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivateAccount {
    #[garde(skip)]
    pub credential_id: CredentialId,
}

/// `PUT /providers/:id/accounts/order`: every account of the entry once, in
/// the order In Order uses and the list shows.
#[derive(Debug, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccountOrder {
    #[garde(skip)]
    pub credential_ids: Vec<CredentialId>,
}

/// `{ active }`: the account the entry now infers with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ActiveAccount {
    pub active: CredentialId,
}

/// `DELETE /providers/:id/accounts/:credentialId`: the account the entry
/// infers with after the removal, null when it has none left.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RemovedAccount {
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<CredentialId>")]
    pub active: Option<CredentialId>,
}

/// `POST /providers/subscription-login`: the first account of a family's
/// entry, by device login.
#[derive(Debug, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubscriptionLogin {
    #[garde(length(min = 1))]
    pub provider_type: String,
    /// The new entry's label; the family's subscription name without it.
    #[serde(default, with = "unwrap_or_skip")]
    #[schemars(with = "Trimmed")]
    #[garde(length(chars, min = 1, max = LABEL_MAX))]
    pub label: Option<Trimmed>,
}

/// `POST /providers/subscription-login/:id/code`: the code the vendor's
/// sign-in page showed, which the login hands to its sign-in. It is one
/// line, since the sign-in reads a line.
#[derive(Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct LoginCode {
    #[garde(length(chars, min = 1, max = CODE_MAX), custom(one_line))]
    pub code: Trimmed,
}

impl std::fmt::Debug for LoginCode {
    /// The code signs the user in, so it is never shown.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("LoginCode { .. }")
    }
}

fn one_line(code: &Trimmed, (): &()) -> garde::Result {
    if code.as_str().chars().any(char::is_control) {
        return Err(garde::Error::new("a code is one line of text"));
    }
    Ok(())
}

/// The answer of a pasted code: its number among the codes the login took,
/// counting from one, which `codesTaken` and a refusal of it then name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CodeTaken {
    pub code: u32,
}

/// The 202 answer of a login start: `{ login: { id, status: "pending" } }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LoginStarted {
    pub login: StartedLogin,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StartedLogin {
    pub id: LoginId,
    pub status: PendingStatus,
}

/// The status of a login that has just started.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PendingStatus {
    Pending,
}

/// `GET /providers/subscription-login/:id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LoginAnswer {
    pub login: LoginState,
}

/// Where a login is; a finished one is kept for ten minutes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "status",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum LoginState {
    /// Running: `stage` says what it waits for. Once it waits for the user,
    /// the user opens the address and enters the code; both are null until
    /// the vendor names them. With `needs_code`, the user signs in at the
    /// address instead and pastes the code the page shows, which the login
    /// now takes, numbering them in `codes_taken`, and `code_error` says why
    /// it refused the last of them, never an earlier one. The
    /// login ends at `expires_at` unless the user finishes it first.
    Pending {
        stage: LoginStage,
        #[serde(deserialize_with = "Option::deserialize")]
        #[schemars(with = "Nullable<String>")]
        verification_url: Option<String>,
        #[serde(deserialize_with = "Option::deserialize")]
        #[schemars(with = "Nullable<String>")]
        user_code: Option<String>,
        needs_code: bool,
        /// How many codes the user pasted the login took.
        codes_taken: u32,
        /// Why the sign-in refused the last code it took, code number
        /// `codes_taken`, in its own words; null until it refuses it, and
        /// again once another is pasted.
        #[serde(deserialize_with = "Option::deserialize")]
        #[schemars(with = "Nullable<String>")]
        code_error: Option<String>,
        /// Null while the login is `preparing`, until it knows its kind and
        /// so its lifetime, which counts from its start.
        #[serde(deserialize_with = "Option::deserialize")]
        #[schemars(with = "Nullable<Timestamp>")]
        expires_at: Option<Timestamp>,
    },
    /// The login stored its account: `credentialId`, which is the entry's
    /// active account only when the entry had none.
    Completed {
        provider_id: ProviderId,
        credential_id: CredentialId,
    },
    /// The user cancelled the login, on any of their pages; nothing was
    /// stored.
    Cancelled,
    /// The login failed or expired: `message` says which, naming the stage
    /// it failed at, and `request_id` is the request that started it, under
    /// which the server log holds its whole course.
    Failed {
        message: String,
        request_id: RequestId,
    },
}

/// One of the user's sign-ins, as the `logins` message and the product
/// state list them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListedLogin {
    pub id: LoginId,
    /// The entry the login adds an account to; null for a login that makes
    /// a new entry.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<ProviderId>")]
    pub provider_id: Option<ProviderId>,
    /// The entry's family, or the family of the entry the login makes.
    pub provider_type: String,
    /// As `GET /providers/subscription-login/:id` answers it.
    pub login: LoginState,
}

/// `GET /models`: the catalog of every entry the caller infers with.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ModelCatalog {
    pub providers: Vec<CatalogProvider>,
    /// The selection Automatic resolves to for the small model
    /// (`crates/backend-providers/src/llm/small_model.rs`), as for a conversation on the first
    /// entry; null when Only the Conversation's Provider is on or no tier
    /// matches.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<ModelSelection>")]
    pub small_model: Option<ModelSelection>,
}

/// One entry's catalog with the provider's health, read when the backend
/// answers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CatalogProvider {
    pub provider_id: ProviderId,
    pub display_name: String,
    /// The command package that installs the CLI the provider's requests
    /// start on the Claude Code machine; null when its requests are HTTP.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    pub cli_package: Option<String>,
    pub models: Vec<CatalogModel>,
    /// The image models the entry offers, newest first: a vendor entry's
    /// vendor's as the media catalog describes them, a compatible entry's
    /// those its user added; listed whether or not its Image Generation is
    /// on.
    pub image_models: Vec<EntryMediaModel>,
    /// The video models the entry offers, as `image_models` lists its
    /// image models.
    pub video_models: Vec<EntryMediaModel>,
    /// The audio models the entry offers, each with its kinds, as
    /// `image_models` lists its image models.
    pub audio_models: Vec<EntryMediaModel>,
    /// When the source was last downloaded; the Unix epoch for a configured
    /// list, and for a catalog no refresh has filled.
    pub source_fetched_at: Timestamp,
    /// Whether this is a copy kept after a failed refresh.
    pub stale: bool,
    pub warnings: Vec<String>,
    pub auth: AuthState,
    pub runtime: RuntimeState,
    pub availability: Availability,
}

/// A media model of an entry as Settings › Providers lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct EntryMediaModel {
    pub id: String,
    pub name: String,
    /// The release date, `YYYY-MM-DD`; null when neither the catalog nor
    /// the vendor states one.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    pub released: Option<String>,
    /// For an audio model, what it makes, speech, music or sound effects,
    /// as the catalog or its entry's shape says; empty for an image or a
    /// video model, and for an audio model nothing says it of.
    pub kinds: Vec<AudioKind>,
}

/// A catalog model with the selection the backend built from it, so the
/// web app never converts one itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CatalogModel {
    #[serde(flatten)]
    pub model: ProviderModel,
    pub selection: ModelSelection,
}

/// Whether the entry's models can be used now: not while the entry is
/// incomplete, nor while its health says it cannot run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Availability {
    Available {},
    Unavailable {
        reason: UnavailableReason,
        message: String,
        /// What an incomplete entry lacks, in this order: present for the
        /// reason `setup` alone, from which the page marks its fields.
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        #[schemars(with = "Vec<SetupPart>")]
        missing: Option<Vec<SetupPart>>,
    },
}

/// What makes an entry's models unavailable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UnavailableReason {
    /// The entry lacks a part it needs to infer
    /// (`crates/backend-user-shard/src/provider_settings.rs`).
    Setup,
    /// The credential is refused, or a subscription's account expired.
    Authentication,
    /// The provider cannot run requests.
    Runtime,
}

/// A part an incomplete entry lacks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum SetupPart {
    /// A vendor entry's key.
    ApiKey,
    /// A compatible entry's endpoint.
    Endpoint,
    /// A model, on an entry whose configured list is empty.
    Models,
    /// A subscription entry's account.
    Account,
}

/// The reason an incomplete entry's models cannot be chosen.
pub const SETUP_MESSAGE: &str = "Finish setting up this provider in Settings";

impl Availability {
    /// An incomplete entry's availability, lacking `missing`.
    pub fn setup(missing: Vec<SetupPart>) -> Self {
        Self::Unavailable {
            reason: UnavailableReason::Setup,
            message: SETUP_MESSAGE.to_owned(),
            missing: Some(missing),
        }
    }
}

/// `{ results }`: at most 20 matches, newest release first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ModelSearchResults {
    pub results: Vec<ModelMatch>,
}

/// One models.dev model, filled as a configured model would be; a fact
/// models.dev does not state is null or empty for the user to fill.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ModelMatch {
    /// `<vendor>/<model id>`, such as `google/gemini-3.8-flash`.
    #[serde(rename = "ref")]
    pub reference: String,
    pub display_name: String,
    /// The model's id as `vendor` names it.
    pub id: String,
    /// The models.dev vendor the model is filled from.
    pub vendor: String,
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<u32>")]
    pub context_window: Option<u32>,
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<u32>")]
    pub output_limit: Option<u32>,
    /// Its effort levels; empty for a model without thinking, or one whose
    /// efforts models.dev does not state.
    pub thinking_efforts: Vec<String>,
    /// The types the model reads natively: `[]` for none, null when unknown.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<Vec<FileExtension>>")]
    pub accepted_extensions: Option<Vec<FileExtension>>,
    /// The release date as models.dev states it, such as `2026-06-17`.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    pub release_date: Option<String>,
}

/// `GET /providers/:id/cli`: the command-line tool of an entry whose
/// provider runs one on the Claude Code machine. Reading it wakes nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProviderCli {
    pub newest: NewestVersion,
    /// The caller's last install on the Claude Code machine that no
    /// conversation asked for, since the backend started; null before one.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<CliInstall>")]
    pub install: Option<CliInstall>,
}

/// The vendor's newest version, or why it could not be read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum NewestVersion {
    Read { version: String },
    Unreadable { message: String },
}

/// Where an install of the tool stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "state",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum CliInstall {
    Installing {},
    /// `path` is the executable on the Claude Code machine.
    Installed {
        path: String,
    },
    Failed {
        message: String,
    },
}

/// The 202 answer of `POST /providers/:id/cli/install`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CliInstallAnswer {
    pub install: CliInstall,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn model(value: serde_json::Value) -> Result<ConfiguredModel, String> {
        let model: ConfiguredModel =
            serde_json::from_value(value).map_err(|error| error.to_string())?;
        model.validate().map_err(|report| report.to_string())?;
        Ok(model)
    }

    fn configured() -> serde_json::Value {
        json!({
            "id": " gpt-5.5 ",
            "displayName": "GPT-5.5",
            "contextWindow": 272_000,
            "outputLimit": null,
            "thinkingEfforts": ["low", "high"],
            "acceptedExtensions": ["png", "pdf"],
            "fastTier": "priority"
        })
    }

    #[test]
    fn a_configured_model_states_its_facts_within_their_bounds() {
        assert_eq!(model(configured()).unwrap().id.as_str(), "gpt-5.5");
        let refused = [
            ("contextWindow", json!(0)),
            ("outputLimit", json!(0)),
            ("outputLimit", json!(272_001)),
            ("id", json!("   ")),
            ("thinkingEfforts", json!([""])),
            ("acceptedExtensions", json!([".png"])),
            ("fastTier", json!("")),
        ];
        for (field, value) in refused {
            let mut body = configured();
            body[field] = value.clone();
            assert!(model(body).is_err(), "{field}: {value}");
        }
        let mut absent = configured();
        absent.as_object_mut().unwrap().remove("outputLimit");
        assert!(model(absent).is_err(), "a nullable field must be present");
        let mut unknown = configured();
        unknown["cost"] = json!(1);
        assert!(model(unknown).is_err());
    }

    #[test]
    fn a_model_list_holds_up_to_a_thousand_models() {
        let list = |models: Vec<serde_json::Value>| {
            serde_json::from_value::<ConfiguredModels>(json!(models)).unwrap()
        };
        assert!(list(vec![configured()]).validate().is_ok());
        // An empty list is an entry waiting for its first model.
        assert!(list(Vec::new()).validate().is_ok());
    }

    #[test]
    fn a_new_entry_names_its_source_and_a_patch_tells_null_from_absent() {
        let vendor: CreateProvider = serde_json::from_value(json!({
            "source": "vendor", "vendorId": "deepseek", "label": " DeepSeek ", "apiKey": "sk-1"
        }))
        .unwrap();
        assert!(
            matches!(vendor, CreateProvider::Vendor { label: Some(ref label), .. } if label.as_str() == "DeepSeek")
        );
        // Everything but the kind may come later.
        let bare: CreateProvider =
            serde_json::from_value(json!({ "source": "vendor", "vendorId": "google" })).unwrap();
        assert!(matches!(
            bare,
            CreateProvider::Vendor { label: None, api_key: None, .. }
        ));
        for refused in [
            json!({ "vendorId": "deepseek", "label": "x", "apiKey": "k" }),
            json!({ "source": "vendor", "vendorId": "deepseek", "apiKey": null }),
            json!({ "source": "vendor", "vendorId": "deepseek", "label": "x", "apiKey": "k", "wireApi": "responses" }),
            json!({ "source": "compatible", "label": "x", "apiKey": "k", "baseUrl": null }),
            json!({ "source": "compatible", "label": "x", "apiKey": "k", "baseUrl": "gw.example" }),
            json!({ "source": "family", "providerType": "openai", "baseUrl": "https://gw.example/v1" }),
        ] {
            assert!(
                serde_json::from_value::<CreateProvider>(refused.clone()).is_err(),
                "{refused}"
            );
        }
        // A compatible entry may be created with nothing but its kind.
        let compatible: CreateProvider =
            serde_json::from_value(json!({ "source": "compatible" })).unwrap();
        assert!(matches!(
            compatible,
            CreateProvider::Compatible { provider_type: None, base_url: None, api_key: None, .. }
        ));
        let patch: ProviderPatch = serde_json::from_value(
            json!({ "baseUrl": null, "apiKey": null, "label": "Work" }),
        )
        .unwrap();
        assert_eq!(
            (patch.base_url, patch.api_key, patch.models.is_none()),
            (Some(None), Some(None), true)
        );
        assert_eq!(patch.label.unwrap().as_str(), "Work");
    }
}

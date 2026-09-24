//! Provider entries, their accounts and logins, and the account-wide model
//! catalog (`web-api.md` § Model configuration and provider inspection, §
//! Subscription accounts). No body the backend answers carries key or token
//! material.

use demi_core::{
    AccountInfo, AuthState, FileExtension, ModelSelection, Nullable, ProviderModel, QuotaSnapshot, RuntimeState,
    Timestamp, WireApi,
};
use garde::Validate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::rust::{double_option, unwrap_or_skip};

use crate::ids::{CredentialId, DeviceId, LoginId, ProviderId};
use crate::text::{EndpointUrl, Trimmed};

/// The most characters (Unicode scalar values) an entry's label has, after
/// trimming.
pub const LABEL_MAX: usize = 80;

/// The most characters (Unicode scalar values) a pasted setup token has,
/// after trimming.
pub const TOKEN_MAX: usize = 16384;

/// How an entry authenticates (`providers.md` § Families, vendors and
/// endpoints).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CredentialKind {
    /// A key the user types in.
    ApiKey,
    /// Accounts from a device login or a setup-token import.
    Subscription,
}

serde_plain::derive_display_from_serialize!(CredentialKind);
serde_plain::derive_fromstr_from_deserialize!(CredentialKind);

/// One model of an entry's configured list, which states its facts directly
/// (`models.md` § Catalog sources): its first thinking effort is its
/// default, its Fast tier, when it names one, is its only service tier, and
/// it is taken to call tools.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfiguredModel {
    #[garde(length(chars, min = 1, max = 256))]
    pub id: Trimmed,
    #[garde(length(chars, min = 1, max = 256))]
    pub display_name: Trimmed,
    /// Tokens.
    #[garde(range(min = 1))]
    pub context_window: u32,
    /// Null for no model-specific limit; never beyond the context window.
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

/// An output limit is at most the context window.
fn within(context_window: u32) -> impl FnOnce(&Option<u32>, &()) -> garde::Result {
    move |output_limit, ()| match output_limit {
        Some(limit) if *limit > context_window => Err(garde::Error::new("the output limit exceeds the context window")),
        _ => Ok(()),
    }
}

/// An entry's complete manual model list: 1 to 1000 models with distinct
/// ids.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(transparent)]
#[garde(transparent)]
pub struct ConfiguredModels(#[garde(length(min = 1, max = 1000), dive, custom(distinct_ids))] pub Vec<ConfiguredModel>);

fn distinct_ids(models: &[ConfiguredModel], (): &()) -> garde::Result {
    let mut seen = std::collections::HashSet::new();
    match models.iter().find(|model| !seen.insert(model.id.as_str())) {
        Some(repeated) => Err(garde::Error::new(format!(
            "the model id {:?} appears twice",
            repeated.id.as_str()
        ))),
        None => Ok(()),
    }
}

/// `POST /providers`: an API-key entry from the vendor list, whose family,
/// wire and endpoint the vendor supplies, or a custom endpoint that names its
/// family itself.
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
        #[garde(length(chars, min = 1, max = LABEL_MAX))]
        label: Trimmed,
        #[garde(length(min = 1))]
        api_key: String,
        /// Replaces the vendor's endpoint.
        #[serde(default, with = "unwrap_or_skip")]
        #[schemars(with = "EndpointUrl")]
        #[garde(skip)]
        base_url: Option<EndpointUrl>,
        /// Replaces the vendor's live model list.
        #[serde(default, with = "unwrap_or_skip")]
        #[schemars(with = "ConfiguredModels")]
        #[garde(dive)]
        models: Option<ConfiguredModels>,
    },
    Custom {
        #[garde(length(min = 1))]
        provider_type: String,
        /// For the `openai` family only; its default is Responses.
        #[serde(default, with = "unwrap_or_skip")]
        #[schemars(with = "WireApi")]
        #[garde(skip)]
        wire_api: Option<WireApi>,
        #[garde(length(chars, min = 1, max = LABEL_MAX))]
        label: Trimmed,
        #[garde(length(min = 1))]
        api_key: String,
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
/// also a new key, endpoint and model list, where `null` removes the
/// endpoint override or returns to the live model list.
#[derive(Debug, Default, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderPatch {
    #[serde(default, with = "unwrap_or_skip")]
    #[schemars(with = "Trimmed")]
    #[garde(length(chars, min = 1, max = LABEL_MAX))]
    pub label: Option<Trimmed>,
    #[serde(default, with = "unwrap_or_skip")]
    #[schemars(with = "String")]
    #[garde(length(min = 1))]
    pub api_key: Option<String>,
    #[serde(default, with = "double_option")]
    #[schemars(with = "Option<EndpointUrl>")]
    #[garde(skip)]
    pub base_url: Option<Option<EndpointUrl>>,
    #[serde(default, with = "double_option")]
    #[schemars(with = "Option<ConfiguredModels>")]
    #[garde(dive)]
    pub models: Option<Option<ConfiguredModels>>,
}

/// An entry as the browser sees it: its family, label, endpoint, vendor and
/// model list, never its key.
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
    /// The vendor an entry was added from; null for a custom endpoint and a
    /// subscription.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    pub vendor_id: Option<String>,
    /// The configured endpoint; null for the family's default.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<EndpointUrl>")]
    pub base_url: Option<EndpointUrl>,
    /// The configured model list; null for the entry's live catalog.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<ConfiguredModels>")]
    pub models: Option<ConfiguredModels>,
    pub created_at: Timestamp,
}

/// `{ provider }`: the answer of a create, an edit and a setup-token import.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProviderAnswer {
    pub provider: ProviderDto,
}

/// `GET /providers`: the entries of the caller's scope, oldest first.
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
    /// Whether the provider runs a process on the user's Cloud.
    pub requires_process_capable_host: bool,
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
    #[garde(length(min = 1))]
    pub model_id: String,
    #[serde(default, with = "unwrap_or_skip")]
    #[schemars(with = "CredentialId")]
    #[garde(skip)]
    pub credential_id: Option<CredentialId>,
}

/// What a test found. A test that ran and failed is a result, with the
/// provider's own reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum TestResult {
    /// The model answered; `model` is its display name.
    Passed { model: String },
    Failed {
        message: String,
        /// The model's display name, when the catalog lists it.
        #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
        #[schemars(with = "String")]
        model: Option<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
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

/// `POST /providers/setup-token`: a Claude Code entry from a token that
/// `claude setup-token` printed.
#[derive(Debug, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct SetupTokenImport {
    #[garde(length(chars, min = 1, max = TOKEN_MAX))]
    pub token: Trimmed,
    #[garde(length(chars, min = 1, max = LABEL_MAX))]
    pub label: Trimmed,
}

/// `POST /providers/:id/accounts`: another account of an entry, from a setup
/// token.
#[derive(Debug, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct AddToken {
    #[garde(length(chars, min = 1, max = TOKEN_MAX))]
    pub token: Trimmed,
}

/// `{ account }`: the account a token import added.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AddedAccount {
    pub account: AccountInfo,
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

/// `{ active }`: the account the entry now infers with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ActiveAccount {
    pub active: CredentialId,
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

/// Where a device login is; a finished one is kept for ten minutes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum LoginState {
    /// Waiting for the user, who opens the address and enters the code;
    /// both are null until the vendor names them.
    Pending {
        #[serde(deserialize_with = "Option::deserialize")]
        #[schemars(with = "Nullable<String>")]
        verification_url: Option<String>,
        #[serde(deserialize_with = "Option::deserialize")]
        #[schemars(with = "Nullable<String>")]
        user_code: Option<String>,
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
    Failed {
        message: String,
    },
}

/// `GET /models`: the catalog of every entry the caller infers with.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ModelCatalog {
    pub providers: Vec<CatalogProvider>,
}

/// One entry's catalog with the provider's health, read when the backend
/// answers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CatalogProvider {
    pub provider_id: ProviderId,
    pub display_name: String,
    pub requires_process_capable_host: bool,
    pub models: Vec<CatalogModel>,
    /// When the source was last downloaded; the Unix epoch for a configured
    /// or built-in list, and for a catalog no refresh has filled.
    pub source_fetched_at: Timestamp,
    /// Whether this is a copy kept after a failed refresh.
    pub stale: bool,
    pub warnings: Vec<String>,
    pub auth: AuthState,
    pub runtime: RuntimeState,
    pub availability: Availability,
}

/// A catalog model with the selection the backend built from it, so the
/// browser never converts one itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CatalogModel {
    #[serde(flatten)]
    pub model: ProviderModel,
    pub selection: ModelSelection,
}

/// Whether the entry's models can be used now, from its health.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Availability {
    Available {},
    Unavailable { reason: UnavailableReason, message: String },
}

/// What makes an entry's models unavailable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UnavailableReason {
    /// The credential is missing or refused.
    Authentication,
    /// The provider cannot run requests.
    Runtime,
}

/// `GET /providers/:id/cli`: the command-line tool of an entry whose
/// provider runs one on the user's Cloud. Reading it wakes nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProviderCli {
    pub newest: NewestVersion,
    /// The last install on the caller's Cloud that no conversation asked
    /// for, since the backend started; null before one.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<CliInstall>")]
    pub install: Option<CliInstall>,
    /// The caller's Cloud while its runner is connected; empty otherwise.
    pub machines: Vec<CliMachine>,
}

/// The vendor's newest version, or why it could not be read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum NewestVersion {
    Read { version: String },
    Unreadable { message: String },
}

/// Where an install of the tool stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "state", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum CliInstall {
    Installing {},
    /// `path` is the executable on the Cloud.
    Installed { path: String },
    Failed { message: String },
}

/// A machine the tool runs on, with the versions it has, newest first;
/// null when it did not answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CliMachine {
    pub device_id: DeviceId,
    pub name: String,
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<Vec<String>>")]
    pub versions: Option<Vec<String>>,
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
        let model: ConfiguredModel = serde_json::from_value(value).map_err(|error| error.to_string())?;
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
    fn a_model_list_holds_one_to_a_thousand_models_with_distinct_ids() {
        let list = |models: Vec<serde_json::Value>| {
            serde_json::from_value::<ConfiguredModels>(json!(models))
                .unwrap()
                .validate()
        };
        assert!(list(vec![configured()]).is_ok());
        assert!(list(Vec::new()).is_err());
        let twice = list(vec![configured(), configured()]).unwrap_err().to_string();
        assert!(twice.contains("appears twice"), "{twice}");
    }

    #[test]
    fn a_new_entry_names_its_source_and_a_patch_tells_null_from_absent() {
        let vendor: CreateProvider = serde_json::from_value(json!({
            "source": "vendor", "vendorId": "deepseek", "label": " DeepSeek ", "apiKey": "sk-1"
        }))
        .unwrap();
        assert!(matches!(vendor, CreateProvider::Vendor { ref label, .. } if label.as_str() == "DeepSeek"));
        for refused in [
            json!({ "vendorId": "deepseek", "label": "x", "apiKey": "k" }),
            json!({ "source": "vendor", "vendorId": "deepseek", "label": "x", "apiKey": "k", "wireApi": "responses" }),
            json!({ "source": "custom", "providerType": "openai", "label": "x", "apiKey": "k", "baseUrl": null }),
            json!({ "source": "custom", "providerType": "openai", "label": "x", "apiKey": "k", "baseUrl": "gw.example" }),
        ] {
            assert!(
                serde_json::from_value::<CreateProvider>(refused.clone()).is_err(),
                "{refused}"
            );
        }
        let patch: ProviderPatch = serde_json::from_value(json!({ "baseUrl": null, "label": "Work" })).unwrap();
        assert_eq!((patch.base_url, patch.models.is_none()), (Some(None), true));
        assert_eq!(patch.label.unwrap().as_str(), "Work");
    }

    #[test]
    fn a_login_travels_as_its_status_and_a_catalog_model_carries_its_selection_beside_its_facts() {
        let pending = LoginState::Pending {
            verification_url: Some("https://auth.openai.com/codex/device".into()),
            user_code: Some("ABCD-1234".into()),
            expires_at: None,
        };
        assert_eq!(
            serde_json::to_value(&pending).unwrap(),
            json!({
                "status": "pending",
                "verificationUrl": "https://auth.openai.com/codex/device",
                "userCode": "ABCD-1234",
                "expiresAt": null
            })
        );
        let started = LoginStarted {
            login: StartedLogin {
                id: LoginId::try_from("login-1").unwrap(),
                status: PendingStatus::Pending,
            },
        };
        assert_eq!(
            serde_json::to_value(&started).unwrap(),
            json!({ "login": { "id": "login-1", "status": "pending" } })
        );

        let model = ProviderModel {
            id: "m".into(),
            display_name: "M".into(),
            description: None,
            context_window: Some(1000),
            output_limit: None,
            supports_tools: None,
            supports_attachments: None,
            supports_video: None,
            accepted_extensions: None,
            supports_reasoning: None,
            supported_thinking_efforts: None,
            default_thinking_effort: None,
            can_disable_thinking: None,
            service_tiers: Vec::new(),
            default_service_tier_id: None,
            cost: None,
        };
        let catalog = CatalogModel {
            selection: model.selection("p", None, None),
            model,
        };
        let json = serde_json::to_value(&catalog).unwrap();
        assert_eq!(
            (json["id"].clone(), json["selection"]["model"]["id"].clone()),
            (json!("m"), json!("m"))
        );
        assert_eq!(serde_json::from_value::<CatalogModel>(json).unwrap(), catalog);
    }
}

//! The provider families entries are assembled with (`providers.md` §
//! Families, vendors and endpoints): each family says how its entries
//! authenticate and builds the provider of one entry and account from the
//! entry's decoded configuration. The backend starts with its built-in
//! families (`demi_backend::families`); a test registers scripted ones
//! beside them.

use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;

use demi_provider_claude_code::{AccountMachine, Placement};
use demi_provider_common::credentials::{CredentialPool, SubscriptionAccounts};
use demi_provider_common::{RuntimeEnv, VendorPolicy};
use demi_provider_common::models_dev::ModelsDevClient;
use demi_provider_common::images::{ImageMaker, ImageModelInfo};
use demi_provider_common::quota::QuotaSnapshotStore;
use demi_provider_common::{Provider, ProviderRuntime, Secret};
use demi_shared_types::{Clock, ImageShape, WireApi};
use demi_web_api_protocol::providers::CredentialKind;
use url::Url;

/// A provider family: `anthropic`, `codex`, or a test's scripted one.
pub trait ProviderFamily: Send + Sync + 'static {
    /// How the family's entries authenticate.
    fn credential(&self) -> CredentialKind;

    /// The wires an entry of the family may choose; none for a family that
    /// speaks one.
    fn wires(&self) -> &'static [WireApi] {
        &[]
    }

    /// The base URL an entry of the family that names none uses
    /// (`providers.md` § Endpoints); none for a subscription family.
    fn default_endpoint(&self) -> Option<&'static str> {
        None
    }

    /// The provider of one entry and account.
    fn provider(&self, args: FamilyArgs) -> Result<Arc<dyn Provider>, FamilyError>;

    /// How a vendor entry of the family makes images, by the models.dev id
    /// of its vendor, none for a subscription (`media.md` § Which entries
    /// make which media); none when it makes none. A compatible entry's is
    /// the one its user chose, whatever its family.
    fn image_shape(&self, _vendor_id: Option<&str>) -> Option<ImageShape> {
        None
    }

    /// A session's runtime of the provider `args` build, whose process
    /// `placement` starts, with the shard's `env`; none for a family whose
    /// provider runs no process (`claude-code.md` § How a runtime gets its
    /// process).
    fn process_runtime(
        &self,
        _args: FamilyArgs,
        _env: RuntimeEnv,
        _placement: Rc<dyn Placement>,
    ) -> Option<Result<Box<dyn ProviderRuntime>, FamilyError>> {
        None
    }

    /// The account operations of the provider `args` build, whose sign-in
    /// runs on the args' machine; none for a family whose login runs no
    /// process (`claude-code.md` § Accounts and sign-in).
    fn login_accounts(
        &self,
        _args: FamilyArgs,
    ) -> Option<Result<Box<dyn SubscriptionAccounts>, FamilyError>> {
        None
    }
}

/// What a family builds a provider from.
pub struct FamilyArgs {
    /// The entry's id, which a provider that logs its runs names them by.
    pub entry_id: String,
    /// The entry's label.
    pub label: String,
    pub credential: FamilyCredential,
    /// The entry's client on the backend's shared services, for the
    /// provider's own requests such as its directory and quota probes
    /// (`port-exposure.md` § What is shared). A runtime gets the entry's
    /// client on the shard it runs on instead.
    pub http: demi_provider_common::ProviderHttp,
    /// The shared services' client itself, for what a family reads that is
    /// no entry's, such as the newest Codex release.
    pub services_http: reqwest::Client,
    pub clock: Arc<dyn Clock>,
    /// The backend's one models.dev copy, for a family whose directory it
    /// is.
    pub models_dev: ModelsDevClient,
    /// The Claude Code machine, which a process provider's sign-in and usage
    /// probe run on (`claude-code.md` § Where it runs); none before the
    /// backend's shards started, and in a backend without them.
    pub machine: Option<Arc<dyn AccountMachine>>,
}

/// The credential a provider stands for.
pub enum FamilyCredential {
    ApiKey(ApiKeyArgs),
    Subscription(SubscriptionArgs),
}

/// An API-key entry's settings, read only from the entry.
pub struct ApiKeyArgs {
    /// None for a compatible entry without one, whose requests carry none.
    pub api_key: Option<Secret>,
    /// The vendor's API base; the family's default without it.
    pub base_url: Option<Url>,
    pub wire_api: Option<WireApi>,
    /// The request requirements of the vendor the entry was added from,
    /// which each protocol's provider applies.
    pub vendor: VendorPolicy,
    /// The models.dev id of that vendor; none for a compatible entry.
    pub vendor_id: Option<String>,
    /// How the vendor entry makes images, as its family says; none for a
    /// compatible entry, whose images are the registry's
    /// [`CompatibleImages`].
    pub image_shape: Option<ImageShape>,
}

/// What a compatible entry whose image shape is OpenAI Images makes its
/// images with.
pub struct CompatibleImagesArgs {
    /// The entry's name, which its failures name.
    pub name: String,
    pub key: Option<Secret>,
    pub base: Url,
    /// The image models its user added.
    pub models: Vec<ImageModelInfo>,
    pub clock: Arc<dyn Clock>,
}

/// Builds a compatible entry's OpenAI Images role: the OpenAI key's, one
/// implementation for every endpoint that speaks it.
pub type CompatibleImages = Arc<dyn Fn(CompatibleImagesArgs) -> Arc<dyn ImageMaker> + Send + Sync>;

/// A subscription entry's accounts: the pool bound to the entry, and the
/// account the provider stands for, when the entry has one.
pub struct SubscriptionArgs {
    pub pool: Arc<dyn CredentialPool>,
    pub account: Option<AccountBinding>,
}

/// The account a subscription provider stands for, with its quota snapshot.
pub struct AccountBinding {
    pub credential_id: String,
    pub quota: Arc<dyn QuotaSnapshotStore>,
}

/// Why a family could not build a provider.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FamilyError {
    /// The entry's credential is not the family's kind.
    #[error("the entry's credential is not one its family takes")]
    WrongCredential,
    #[error("{0}")]
    Invalid(String),
}

/// The name a family's entry takes when the user gives none, such as
/// `OpenAI` for the `openai` family's own vendor entry; a family without
/// one of its own is named by its id.
pub fn family_name(family: &str) -> &str {
    match family {
        "openai" => "OpenAI",
        "anthropic" => "Anthropic",
        "google" => "Google",
        "codex" => "Codex",
        "claude-code" => "Claude Code",
        "grok-build" => "Grok Build",
        other => other,
    }
}

/// The families of a backend by name.
#[derive(Clone, Default)]
pub struct FamilyRegistry {
    families: BTreeMap<String, Arc<dyn ProviderFamily>>,
    compatible_images: Option<CompatibleImages>,
}

impl FamilyRegistry {
    /// The registry with `family` under `name`, in place of any family of
    /// that name.
    pub fn with(mut self, name: impl Into<String>, family: impl ProviderFamily) -> Self {
        self.families.insert(name.into(), Arc::new(family));
        self
    }

    pub fn get(&self, name: &str) -> Option<&Arc<dyn ProviderFamily>> {
        self.families.get(name)
    }

    /// The registry with `images` making a compatible entry's OpenAI
    /// Images.
    pub fn with_compatible_images(mut self, images: CompatibleImages) -> Self {
        self.compatible_images = Some(images);
        self
    }

    /// What makes a compatible entry's OpenAI Images; none in a registry
    /// that has no family speaking them.
    pub fn compatible_images(&self) -> Option<&CompatibleImages> {
        self.compatible_images.as_ref()
    }

    /// The names of the subscription families, in order.
    pub fn subscriptions(&self) -> impl Iterator<Item = &str> {
        self.families
            .iter()
            .filter(|(_, family)| family.credential() == CredentialKind::Subscription)
            .map(|(name, _)| name.as_str())
    }
}

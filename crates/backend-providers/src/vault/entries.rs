//! Provider entries: each entry's record with its configuration sealed, whose
//! entries a user infers with, and the account records of a subscription entry.
//!
//! The vault opens a configuration when it reads an entry and validates it
//! strictly. A value that does not open or decode is corrupt and is never
//! repaired.
//!
//! Every credential of the product is a record of the control store, sealed
//! under a key derived from the instance secret and bound to its row. Nothing
//! the product authenticates with is a file.
//!
//! The backend never reads a vendor's own login from the machine it runs on:
//! `~/.codex`, `~/.grok`, the Claude Code keychain item, or a vendor token in
//! its environment. Those belong to whoever operates the machine, and a
//! multi-user product must not lend them to a user whose entry has no account.
//!
//! A credential has no owner of its own: it belongs to a provider entry, and
//! every entry belongs to a user. Instance mode only decides whose entries a
//! user infers with. There is no ownerless entry and no second kind of
//! credential.
//!
//! On a shared instance the master's accounts are used by many users on many
//! workers at once, so:
//!
//! - Refreshes rely on the versioned replace of the credential pool.
//! - The vendor's windows describe the account and not a user. Per-user
//!   consumption is the usage ledger's.
//! - A Claude Code sign-in stays on the Claude Code machine.

use std::sync::Arc;

use demi_backend_database::StorageError;
use demi_backend_database::control::ControlService;
use demi_backend_database::providers::{
    CredentialRow, CredentialWrite, MediaSwitches, NewProvider, ProviderRow, ResealConfigs,
};
use demi_backend_page_sync::{Part, SyncRegistry};
use demi_provider_common::Secret;
use demi_provider_common::credentials::{
    AccountMeta, CredentialPool, MemoryCredentialPool, NewAccount, RefreshGates,
    credential_id_for, decode_secret,
};
use demi_shared_types::{AudioShape, ImageShape, MediaKind, MediaShapes, Timestamp, VideoShape, WireApi};
use demi_web_api_protocol::auth::{Role, UserDto};
use demi_web_api_protocol::ids::{CredentialId, ProviderId, UserId};
use demi_web_api_protocol::providers::{
    AudioMedia, ConfiguredModels, CredentialKind, EntryMedia, ImageMedia, ProviderDto, Rotation, SetupPart, VideoMedia,
};
use demi_web_api_protocol::settings::InstanceMode;
use demi_web_api_protocol::text::EndpointUrl;
use garde::Validate;
use serde::{Deserialize, Serialize};
use serde_with::rust::unwrap_or_skip;

use super::pool::VaultCredentialPool;
use super::seal::{self, VaultKey};

/// The credential vault. Cloning it is cheap.
#[derive(Clone)]
pub struct Vault(Arc<Shared>);

struct Shared {
    control: ControlService,
    key: VaultKey,
    mode: InstanceMode,
    /// The master, whose entries a shared instance's users infer with; setup
    /// creates it once and it never changes.
    master: tokio::sync::OnceCell<UserId>,
    /// One refresh at a time for each account of every entry.
    gates: RefreshGates,
    /// The pages' channels, on which each change of an entry is marked.
    sync: SyncRegistry,
    /// The entries whose active account changed, as each changes.
    account_changes: tokio::sync::broadcast::Sender<ProviderId>,
}

/// How many announced account changes a follower may fall behind before it
/// misses some: they come from the user's own clicks in Settings.
const ACCOUNT_CHANGES_KEPT: usize = 64;

/// An account a subscription entry is created with outside a sign-in:
///
/// - The development backend's Codex sign-in, which the master account is
///   created with (`crates/xtask/src/dev/mod.rs`).
/// - A suite's account whose vendor sign-in cannot run.
#[derive(Debug, Clone)]
pub struct SeededAccount {
    /// The entry's family, such as `codex`.
    pub family: String,
    /// The entry's label, such as `Codex`.
    pub label: String,
    pub account: NewAccount,
    /// Where the account came from, as its record names it.
    pub source: String,
}

/// A provider entry, read and decoded.
#[derive(Debug, Clone, PartialEq)]
pub struct ProviderEntry {
    pub id: ProviderId,
    pub owner: UserId,
    /// The entry's family, such as `anthropic` or `codex`.
    pub family: String,
    pub label: String,
    pub credential: EntryCredential,
    pub created_at: Timestamp,
    /// Whether the entry makes images when its provider can: its Image
    /// Generation switch (`crates/backend-user-shard/src/provider_settings.rs`).
    pub image_generation: bool,
    /// Whether the entry makes videos when its provider can: its Video
    /// Generation switch.
    pub video_generation: bool,
    /// Whether the entry makes audio when its provider can: its Audio
    /// Generation switch.
    pub audio_generation: bool,
}

/// How an entry authenticates.
#[derive(Debug, Clone, PartialEq)]
pub enum EntryCredential {
    ApiKey(ApiKeyConfig),
    /// Accounts in the entry's credential records, of which `active` is the
    /// one the entry infers with unless `rotation` gives each agent one of
    /// its own (`crates/backend-providers/src/llm/rotation.rs`).
    Subscription {
        active: Option<CredentialId>,
        rotation: Rotation,
    },
}

/// An API-key entry's configuration: the document the vault seals. A field
/// it does not name is an error, so a misspelled setting is reported rather
/// than ignored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApiKeyConfig {
    /// None until the user saves one: the entry is incomplete meanwhile.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[garde(skip)]
    pub api_key: Option<Secret>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[garde(skip)]
    pub base_url: Option<EndpointUrl>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[garde(skip)]
    pub wire_api: Option<WireApi>,
    /// The vendor of a vendor entry: one models.dev lists, or its family's
    /// own, whose models.dev id is the family's name (`anthropic`,
    /// `openai`, `google`). A compatible entry names none.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[garde(inner(length(min = 1)))]
    pub vendor_id: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[garde(dive)]
    pub models: Option<ConfiguredModels>,
    /// A compatible entry's image shape, which its user chose; none until
    /// then, and always none for a vendor entry, whose shape is its
    /// vendor's.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[garde(skip)]
    pub image_shape: Option<ImageShape>,
    /// A compatible entry's video shape, which its user chose, as its image
    /// shape is.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[garde(skip)]
    pub video_shape: Option<VideoShape>,
    /// A compatible entry's audio shape, which its user chose, as its image
    /// shape is.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[garde(skip)]
    pub audio_shape: Option<AudioShape>,
}

impl ApiKeyConfig {
    /// Whether the entry is a **compatible entry**
    /// (`crates/provider-common/src/endpoint.rs`): it names no vendor.
    ///
    /// - A vendor entry uses its vendor's endpoint and lists its vendor's
    ///   catalog.
    /// - A compatible entry has the endpoint the user gives, none until then,
    ///   and lists only the models the user configures.
    pub fn is_compatible(&self) -> bool {
        self.vendor_id.is_none()
    }
}

/// The format of the sealed configuration document this release writes
/// and reads, which the control schema names. Format 1, the bare
/// configuration that 0.1.21 and the builds before this one sealed, told a
/// compatible entry by its endpoint rather than by its vendor; the control
/// database's migration re-encodes it ([`ConfigReseal`]).
const CONFIG_FORMAT: u32 = 2;

/// The sealed document: the configuration under its format.
#[derive(Serialize)]
struct SealingConfig<'a> {
    format: u32,
    config: &'a ApiKeyConfig,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SealedConfig {
    format: u32,
    config: ApiKeyConfig,
}

impl ProviderEntry {
    pub fn kind(&self) -> CredentialKind {
        match self.credential {
            EntryCredential::ApiKey(_) => CredentialKind::ApiKey,
            EntryCredential::Subscription { .. } => CredentialKind::Subscription,
        }
    }

    /// What the entry lacks to infer, in the order the page lists it
    /// (`crates/backend-user-shard/src/provider_settings.rs`):
    ///
    /// - A vendor entry's key.
    /// - A compatible entry's endpoint. Its key is optional.
    /// - A model, when its catalog is its configured list and the list is
    ///   empty. A compatible entry's catalog always is its configured list.
    /// - A subscription's account.
    ///
    /// A vendor entry's address is never missing: it uses its vendor's. The
    /// result is empty for a complete entry.
    pub fn missing(&self) -> Vec<SetupPart> {
        match &self.credential {
            EntryCredential::ApiKey(config) => [
                (
                    !config.is_compatible() && config.api_key.is_none(),
                    SetupPart::ApiKey,
                ),
                (
                    config.is_compatible() && config.base_url.is_none(),
                    SetupPart::Endpoint,
                ),
                (
                    config
                        .models
                        .as_ref()
                        .map_or(config.is_compatible(), |models| models.0.is_empty()),
                    SetupPart::Models,
                ),
            ]
            .into_iter()
            .filter_map(|(lacks, part)| lacks.then_some(part))
            .collect(),
            EntryCredential::Subscription { active, .. } => match active {
                Some(_) => Vec::new(),
                None => vec![SetupPart::Account],
            },
        }
    }

    /// A subscription entry's active account.
    pub fn active(&self) -> Option<&CredentialId> {
        match &self.credential {
            EntryCredential::ApiKey(_) => None,
            EntryCredential::Subscription { active, .. } => active.as_ref(),
        }
    }

    /// Which account each agent uses; `manual` for an API-key entry.
    pub fn rotation(&self) -> Rotation {
        match &self.credential {
            EntryCredential::ApiKey(_) => Rotation::Manual,
            EntryCredential::Subscription { rotation, .. } => *rotation,
        }
    }

    /// The switch of `kind`.
    pub fn generates(&self, kind: MediaKind) -> bool {
        match kind {
            MediaKind::Image => self.image_generation,
            MediaKind::Video => self.video_generation,
            MediaKind::Audio => self.audio_generation,
        }
    }

    /// The entry as the web app sees it: never its key.
    ///
    /// - `default_endpoint` is its family's default base URL, which a vendor
    ///   entry that stores no endpoint goes to. A compatible entry without one
    ///   goes nowhere yet.
    /// - `shapes` are the ones it makes media in.
    /// - `catalog` is when the media catalog in use was published.
    pub fn dto(&self, default_endpoint: Option<EndpointUrl>, shapes: MediaShapes, catalog: Timestamp) -> ProviderDto {
        let config = match &self.credential {
            EntryCredential::ApiKey(config) => Some(config),
            EntryCredential::Subscription { .. } => None,
        };
        ProviderDto {
            id: self.id.clone(),
            kind: self.kind(),
            provider_type: self.family.clone(),
            label: self.label.clone(),
            wire_api: config.and_then(|config| config.wire_api),
            vendor_id: config.and_then(|config| config.vendor_id.clone()),
            compatible: config.is_some_and(ApiKeyConfig::is_compatible),
            endpoint: config.and_then(|config| match config.is_compatible() {
                true => config.base_url.clone(),
                false => config.base_url.clone().or(default_endpoint),
            }),
            has_api_key: config.is_some_and(|config| config.api_key.is_some()),
            models: config.and_then(|config| config.models.clone()),
            rotation: self.rotation(),
            media: EntryMedia {
                image: ImageMedia {
                    shape: shapes.image,
                    enabled: self.image_generation,
                },
                video: VideoMedia {
                    shape: shapes.video,
                    enabled: self.video_generation,
                },
                audio: AudioMedia {
                    shape: shapes.audio,
                    enabled: self.audio_generation,
                },
                catalog,
            },
            created_at: self.created_at,
        }
    }
}

impl Vault {
    pub fn new(
        control: ControlService,
        key: VaultKey,
        mode: InstanceMode,
        sync: SyncRegistry,
    ) -> Self {
        Self(Arc::new(Shared {
            control,
            key,
            mode,
            master: tokio::sync::OnceCell::new(),
            gates: RefreshGates::new(),
            sync,
            account_changes: tokio::sync::broadcast::Sender::new(ACCOUNT_CHANGES_KEPT),
        }))
    }

    /// Makes `account` the one the entry `entry` infers with, and announces
    /// the change (`crates/provider-claude-code/src/prepared.rs`); `false`
    /// when the entry holds no such account.
    pub async fn set_active_account(
        &self,
        entry: &ProviderId,
        account: CredentialId,
    ) -> Result<bool, StorageError> {
        let selected = self
            .0
            .control
            .set_active_credential(entry.clone(), account)
            .await?;
        if selected {
            self.announce_account_change(entry);
        }
        Ok(selected)
    }

    /// Stores `account` in the entry `entry`, which selects it when the
    /// entry has none, as a sign-in's first account; that selection is an
    /// announced change of the active account. `false` for an entry that no
    /// longer exists.
    pub async fn write_account(
        &self,
        entry: &ProviderId,
        account: CredentialWrite,
    ) -> Result<bool, StorageError> {
        let written = self
            .0
            .control
            .write_credential(entry.clone(), account)
            .await?;
        if written.is_some_and(|written| written.activated) {
            self.announce_account_change(entry);
        }
        Ok(written.is_some())
    }

    /// Removes `account` from `entry` and answers the account active
    /// afterwards; removing the active one is an announced change of it.
    pub async fn remove_account(
        &self,
        entry: &ProviderEntry,
        account: CredentialId,
    ) -> Result<Option<CredentialId>, StorageError> {
        let was_active = entry.active() == Some(&account);
        let active = self
            .0
            .control
            .remove_credential(entry.id.clone(), account)
            .await?;
        if was_active {
            self.announce_account_change(&entry.id);
        }
        Ok(active)
    }

    /// The entries whose active account changes from now on, whoever
    /// changed it: the user in Settings, or a sign-in that made its account
    /// the active one.
    pub fn account_changes(&self) -> tokio::sync::broadcast::Receiver<ProviderId> {
        self.0.account_changes.subscribe()
    }

    fn announce_account_change(&self, entry: &ProviderId) {
        // With no follower, as before any user's shard started, nobody holds
        // a process to close.
        let _ = self.0.account_changes.send(entry.clone());
    }

    pub fn control(&self) -> &ControlService {
        &self.0.control
    }

    pub fn key(&self) -> &VaultKey {
        &self.0.key
    }

    pub fn gates(&self) -> &RefreshGates {
        &self.0.gates
    }

    /// Whether `user` configures the entries of their scope: everyone on an
    /// isolated instance, only the master on a shared one.
    pub fn configures(&self, user: &UserDto) -> bool {
        self.0.mode == InstanceMode::Isolated || user.role == Role::Master
    }

    /// Whose entries `user` infers with: their own on an isolated instance, the
    /// master's on a shared one.
    pub async fn owner_for(&self, user: &UserId) -> Result<UserId, StorageError> {
        if self.0.mode == InstanceMode::Isolated {
            return Ok(user.clone());
        }
        let master = self
            .0
            .master
            .get_or_try_init(|| async {
                match self.0.control.master().await? {
                    Some(master) => Ok(master),
                    // A signed-in user means setup ran, and setup creates
                    // the master.
                    None => Err(StorageError::Corrupt {
                        table: "users",
                        column: "role",
                        reason: "a shared instance has no master account".into(),
                    }),
                }
            })
            .await?;
        Ok(master.clone())
    }

    /// Marks the providers changed on the channels of every user who infers
    /// with the entries of `owner`: every user on a shared instance, the
    /// owner alone on an isolated one.
    pub fn mark_changed(&self, owner: &UserId) {
        match self.0.mode {
            InstanceMode::Shared => self.0.sync.mark_everyone(&Part::Providers),
            InstanceMode::Isolated => self.0.sync.mark(owner, Part::Providers),
        }
    }

    /// Marks the providers changed for every user who infers with the entry
    /// `id`. A lookup that fails is logged: the pages then show the change
    /// with the entry's next one.
    pub async fn mark_entry_changed(&self, id: &ProviderId) {
        if self.0.mode == InstanceMode::Shared {
            self.0.sync.mark_everyone(&Part::Providers);
            return;
        }
        match self.0.control.provider(id.clone()).await {
            Ok(Some(row)) => self.0.sync.mark(&row.owner, Part::Providers),
            // An entry deleted meanwhile marked its owner as it went.
            Ok(None) => {}
            Err(error) => tracing::warn!(
                provider = %id,
                error = &error as &dyn std::error::Error,
                "a change of an entry was not marked on its owner's pages"
            ),
        }
    }

    /// The entry `id` when it is one of `user`'s scope.
    pub async fn visible(
        &self,
        user: &UserId,
        id: &ProviderId,
    ) -> Result<Option<ProviderEntry>, StorageError> {
        let owner = self.owner_for(user).await?;
        let entry = self.entry(id.clone()).await?;
        Ok(entry.filter(|entry| entry.owner == owner))
    }

    /// The owner's entries in the order of Settings › Providers.
    pub async fn entries(&self, owner: UserId) -> Result<Vec<ProviderEntry>, StorageError> {
        let rows = self.0.control.providers(owner).await?;
        rows.into_iter().map(|row| self.decode(row)).collect()
    }

    pub async fn entry(&self, id: ProviderId) -> Result<Option<ProviderEntry>, StorageError> {
        let row = self.0.control.provider(id).await?;
        row.map(|row| self.decode(row)).transpose()
    }

    /// Stores a new API-key entry.
    pub async fn create_api_key(
        &self,
        owner: UserId,
        family: String,
        label: String,
        config: ApiKeyConfig,
    ) -> Result<ProviderEntry, StorageError> {
        let id = new_id();
        let sealed = self.seal_config(&id, &config);
        let provider = NewProvider {
            id,
            owner,
            family,
            kind: CredentialKind::ApiKey,
            label,
            config: Some(sealed),
            active: None,
        };
        let row = self.0.control.insert_provider(provider, Vec::new()).await?;
        // An API-key entry is under no uniqueness rule.
        let row = row.expect("an API-key entry is always stored");
        self.mark_changed(&row.owner);
        self.decode(row)
    }

    /// Publishes a completed login's entry with the accounts of its staged
    /// pool, in one transaction
    /// (`crates/backend-providers/src/vault/logins.rs`).
    ///
    /// The transaction stores the entry, its accounts, and the staged active
    /// account, or its first. The result is `None` when the owner already holds
    /// the family's subscription entry, and then nothing is stored.
    pub async fn create_subscription(
        &self,
        owner: UserId,
        family: String,
        label: String,
        staged: &MemoryCredentialPool,
    ) -> Result<Option<ProviderEntry>, StorageError> {
        let id = new_id();
        let mut accounts = Vec::new();
        for (meta, secret) in staged.entries() {
            let account =
                CredentialId::try_from(meta.id).map_err(|error| StorageError::Corrupt {
                    table: "provider_credentials",
                    column: "id",
                    reason: format!("a login staged an account whose id is invalid: {error}"),
                })?;
            accounts.push(CredentialWrite {
                secret: secret.map(|secret| self.seal_secret(&id, &account, &secret)),
                id: account,
                identity_key: meta.identity_key,
                label: meta.label,
                detail: meta.detail,
                source: meta.source,
            });
        }
        // A pool held in memory never fails to answer.
        let staged_active = staged.active().await.ok().flatten();
        let active = staged_active
            .and_then(|active| {
                accounts
                    .iter()
                    .find(|account| account.id.as_str() == active)
            })
            .or(accounts.first())
            .map(|account| account.id.clone());
        let provider = NewProvider {
            id,
            owner,
            family,
            kind: CredentialKind::Subscription,
            label,
            config: None,
            active,
        };
        let row = self.0.control.insert_provider(provider, accounts).await?;
        if let Some(row) = &row {
            self.mark_changed(&row.owner);
        }
        row.map(|row| self.decode(row)).transpose()
    }

    /// Publishes `seeded` as the one account of a new entry of `owner`'s,
    /// as a sign-in would have published it; `None` when the owner already
    /// holds the family's subscription entry.
    pub async fn seed(
        &self,
        owner: UserId,
        seeded: &SeededAccount,
        now: Timestamp,
    ) -> Result<Option<ProviderEntry>, StorageError> {
        let NewAccount { secret, label } = seeded.account.clone();
        let id = credential_id_for(label.identity_key.as_deref(), &label.label);
        let meta = AccountMeta {
            id: id.clone(),
            label: label.label,
            detail: label.detail,
            updated_at: now,
            source: seeded.source.clone(),
            identity_key: label.identity_key,
            signed_out: false,
        };
        let staged = MemoryCredentialPool::new();
        // A pool held in memory never fails to answer.
        staged.write(meta, secret).await.expect("a pool in memory stores");
        staged.set_active(&id).await.expect("a pool in memory selects");
        self.create_subscription(owner, seeded.family.clone(), seeded.label.clone(), &staged)
            .await
    }

    /// Replaces the entry's label, family, configuration, rotation or media
    /// switches; `None` for an entry that no longer exists.
    pub async fn update(
        &self,
        id: ProviderId,
        label: Option<String>,
        family: Option<String>,
        config: Option<ApiKeyConfig>,
        rotation: Option<Rotation>,
        switches: MediaSwitches,
    ) -> Result<Option<ProviderEntry>, StorageError> {
        let sealed = config.map(|config| self.seal_config(&id, &config));
        let row = self
            .0
            .control
            .update_provider(id, label, family, sealed, rotation, switches)
            .await?;
        if let Some(row) = &row {
            self.mark_changed(&row.owner);
        }
        row.map(|row| self.decode(row)).transpose()
    }

    /// Puts the owner's entries in the order `ids` names them; `false`,
    /// changing nothing, unless it names each of them once.
    pub async fn set_order(&self, owner: &UserId, ids: Vec<ProviderId>) -> Result<bool, StorageError> {
        let ordered = self.0.control.set_provider_order(owner.clone(), ids).await?;
        if ordered {
            self.mark_changed(owner);
        }
        Ok(ordered)
    }

    /// Deletes the entry with its accounts and catalog record.
    pub async fn delete(&self, entry: &ProviderEntry) -> Result<(), StorageError> {
        self.0.control.delete_provider(entry.id.clone()).await?;
        self.mark_changed(&entry.owner);
        Ok(())
    }

    /// The pool of entry `id`'s accounts, which can reach no other entry's.
    pub fn pool(&self, id: ProviderId) -> Arc<dyn CredentialPool> {
        Arc::new(VaultCredentialPool::new(self.clone(), id))
    }

    /// The entry's account records in the list's order, their secrets
    /// unopened.
    pub async fn accounts(&self, id: ProviderId) -> Result<Vec<CredentialRow>, StorageError> {
        self.0.control.credentials(id).await
    }

    pub async fn account(
        &self,
        id: ProviderId,
        account: CredentialId,
    ) -> Result<Option<CredentialRow>, StorageError> {
        self.0.control.credential(id, account).await
    }

    pub fn seal_secret(
        &self,
        provider: &ProviderId,
        account: &CredentialId,
        secret: &str,
    ) -> Vec<u8> {
        self.0
            .key
            .seal(seal::Row::Secret(provider, account), secret.as_bytes())
    }

    fn seal_config(&self, id: &ProviderId, config: &ApiKeyConfig) -> Vec<u8> {
        seal_config(&self.0.key, id, config)
    }

    fn decode(&self, row: ProviderRow) -> Result<ProviderEntry, StorageError> {
        let credential = match (row.kind, row.config) {
            (CredentialKind::ApiKey, Some(sealed)) => {
                EntryCredential::ApiKey(self.open_config(&row.id, &sealed)?)
            }
            (CredentialKind::Subscription, None) => {
                EntryCredential::Subscription {
                    active: row.active,
                    rotation: row.rotation,
                }
            }
            // The schema ties the configuration to the kind.
            _ => {
                return Err(corrupt_config(
                    "the configuration does not match the entry's kind",
                ));
            }
        };
        Ok(ProviderEntry {
            id: row.id,
            owner: row.owner,
            family: row.family,
            label: row.label,
            credential,
            created_at: row.created_at,
            image_generation: row.image_generation,
            video_generation: row.video_generation,
            audio_generation: row.audio_generation,
        })
    }

    /// The configuration sealed for entry `id`: it must open and hold a
    /// configuration of this release's format that keeps its rules
    /// ([`checked`]). A failure names the field and the kind of fault, never
    /// a value, which may be the key.
    fn open_config(&self, id: &ProviderId, sealed: &[u8]) -> Result<ApiKeyConfig, StorageError> {
        let text = open_text(&self.0.key, id, sealed).map_err(corrupt_config)?;
        let sealed: SealedConfig =
            decode_secret(&text).map_err(|error| corrupt_config(error.to_string()))?;
        if sealed.format != CONFIG_FORMAT {
            return Err(corrupt_config(format!(
                "the configuration's format {} is not {CONFIG_FORMAT}",
                sealed.format
            )));
        }
        checked(sealed.config).map_err(corrupt_config)
    }
}

/// The vault's side of the control database's migration to provider
/// configuration format 2 (`crates/backend-database/src/lib.rs`).
///
/// It opens each configuration of format 1 under the vault's key, decodes and
/// checks it as any configuration is, and seals it again in format 2.
///
/// - A configuration that names neither a vendor nor an endpoint was its
///   family's own vendor's entry and now names that vendor.
/// - A configuration with an endpoint and no vendor stays a compatible entry.
pub struct ConfigReseal(VaultKey);

impl ConfigReseal {
    pub fn new(key: VaultKey) -> Self {
        Self(key)
    }
}

impl ResealConfigs for ConfigReseal {
    fn provider_config(&self, id: &ProviderId, family: &str, sealed: &[u8]) -> Result<Vec<u8>, String> {
        let text = open_text(&self.0, id, sealed)?;
        let config = decode_secret::<ApiKeyConfig>(&text).map_err(|error| error.to_string())?;
        let mut config = checked(config)?;
        if config.vendor_id.is_none() && config.base_url.is_none() {
            config.vendor_id = Some(family.to_owned());
        }
        Ok(seal_config(&self.0, id, &config))
    }
}

/// Entry `id`'s configuration sealed under `key` in this release's format.
fn seal_config(key: &VaultKey, id: &ProviderId, config: &ApiKeyConfig) -> Vec<u8> {
    // A configuration is strings, numbers and lists, which always serialize.
    let document = serde_json::to_vec(&SealingConfig {
        format: CONFIG_FORMAT,
        config,
    })
    .expect("an entry's configuration serializes");
    key.seal(seal::Row::Config(id), &document)
}

/// The text of the configuration sealed for entry `id` under `key`: it must
/// open and be UTF-8.
fn open_text(key: &VaultKey, id: &ProviderId, sealed: &[u8]) -> Result<String, String> {
    let document = key
        .open(seal::Row::Config(id), sealed)
        .map_err(|error| error.to_string())?;
    String::from_utf8(document).map_err(|_| "the configuration is not UTF-8".to_owned())
}

/// `config` when it keeps its rules, its model ids distinct among them.
fn checked(config: ApiKeyConfig) -> Result<ApiKeyConfig, String> {
    config.validate().map_err(|report| report.to_string())?;
    if config
        .models
        .as_ref()
        .is_some_and(|models| models.repeated_id().is_some())
    {
        return Err("a model id appears twice".to_owned());
    }
    Ok(config)
}

fn corrupt_config(reason: impl Into<String>) -> StorageError {
    StorageError::Corrupt {
        table: "providers",
        column: "config",
        reason: reason.into(),
    }
}

/// A new entry's id.
fn new_id() -> ProviderId {
    ProviderId::try_from(uuid::Uuid::new_v4().to_string()).expect("a UUID is not empty")
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use demi_backend_database::control::{ControlService, testing};
    use demi_backend_database::providers::NoSealedConfigs;
    use demi_backend_database::{DatabaseKind, SchemaFit, schema_fit, schema_testing};
    use demi_web_api_protocol::settings::InstanceMode;

    use super::*;
    use crate::vault::seal::VaultKey;

    /// A control database of the schema before provider configuration format 2,
    /// its entries sealed as 0.1.21 sealed them, upgrades as the backend opens
    /// it:
    ///
    /// - The family's own vendor's entry names its vendor.
    /// - An endpoint without a vendor stays a compatible entry.
    ///
    /// One whose configuration does not open under the vault's key stops the
    /// open and still records the earlier schema.
    ///
    /// Planted defects this catches:
    ///
    /// - Format 1 decoded as format 2.
    /// - A family's own entry left naming no vendor, which reads as a
    ///   compatible entry without an endpoint.
    ///
    /// Cost: two small databases, a few milliseconds.
    #[tokio::test]
    async fn a_released_database_names_its_vendors_as_it_upgrades() {
        let key = [3; 32];
        let released = |unreadable: bool| async move {
            let data = tempfile::tempdir().unwrap();
            let path = data.path().join("control.sqlite");
            let clock = Arc::new(demi_shared_types::SystemClock);
            let control = ControlService::open(&path, clock, Arc::new(NoSealedConfigs))
                .await
                .unwrap();
            let master = testing::master(&control).await;
            let entry = |family: &str, sealed_with: [u8; 32], document: serde_json::Value| {
                let id = new_id();
                let sealed = VaultKey::new(sealed_with)
                    .seal(seal::Row::Config(&id), document.to_string().as_bytes());
                NewProvider {
                    id,
                    owner: master.id.clone(),
                    family: family.into(),
                    kind: CredentialKind::ApiKey,
                    label: family.into(),
                    config: Some(sealed),
                    active: None,
                }
            };
            let own = entry("anthropic", key, serde_json::json!({ "apiKey": "sk-1" }));
            let proxy = entry(
                "openai",
                key,
                serde_json::json!({ "apiKey": "sk-2", "baseUrl": "https://proxy.example/v1", "wireApi": "chat-completions" }),
            );
            let mut entries = vec![own.clone(), proxy.clone()];
            if unreadable {
                entries.push(entry("google", [4; 32], serde_json::json!({ "apiKey": "sk-3" })));
            }
            for entry in entries {
                control.insert_provider(entry, Vec::new()).await.unwrap();
            }
            control.close().await.unwrap();
            // The schema before format 2, which the one before account
            // rotation, the one before a placeholder title held no words, the
            // one before interrupted turns were marked, the one before a
            // conversation kept the hosts it may reach, the one before
            // shares, the one before a device's mark had a number,
            // 0.1.22-dev.20261015T105826Z's, 0.1.22-dev.20261015T134426Z's,
            // 0.1.22-dev.20261016T043704Z's, 0.1.22-dev.20261016T095626Z's,
            // 0.1.22's, 0.1.25's, 0.1.27's, 0.1.32's, 0.1.36's and 0.1.37's
            // follow in the history.
            schema_testing::store_as_shipped_before(&path, DatabaseKind::Control, 16);
            (data, path, own.id, proxy.id)
        };
        let clock = Arc::new(demi_shared_types::SystemClock);

        let (_data, path, own, proxy) = released(false).await;
        let control = ControlService::open(&path, clock.clone(), Arc::new(ConfigReseal::new(VaultKey::new(key))))
            .await
            .unwrap();
        let vault = Vault::new(
            control.clone(),
            VaultKey::new(key),
            InstanceMode::Isolated,
            demi_backend_page_sync::SyncRegistry::default(),
        );
        let config = |entry: ProviderEntry| match entry.credential {
            EntryCredential::ApiKey(config) => config,
            EntryCredential::Subscription { .. } => panic!("an API-key entry"),
        };
        let own_config = config(vault.entry(own).await.unwrap().unwrap());
        assert_eq!(
            (own_config.vendor_id.as_deref(), own_config.is_compatible(), own_config.base_url),
            (Some("anthropic"), false, None)
        );
        let proxy_config = config(vault.entry(proxy).await.unwrap().unwrap());
        assert_eq!(
            (
                proxy_config.is_compatible(),
                proxy_config.base_url.map(|url| url.as_str().to_owned()),
                proxy_config.wire_api,
            ),
            (true, Some("https://proxy.example/v1".to_owned()), Some(WireApi::ChatCompletions))
        );
        control.close().await.unwrap();

        let (_data, path, _, _) = released(true).await;
        let refused = ControlService::open(&path, clock, Arc::new(ConfigReseal::new(VaultKey::new(key)))).await;
        assert!(matches!(refused, Err(StorageError::Migration { .. })), "{:?}", refused.err());
        assert!(matches!(schema_fit(&path, DatabaseKind::Control), Ok(SchemaFit::Migrates)));
    }
}

//! Provider entries (`providers.md` § Credential vault, § Scope): each
//! entry's record with its configuration sealed, whose entries a user infers
//! with, and the account records of a subscription entry. The vault opens a
//! configuration when it reads an entry and validates it strictly; a value
//! that does not open or decode is corrupt and is never repaired.

use std::sync::Arc;

use demi_backend_database::StorageError;
use demi_backend_database::control::ControlService;
use demi_backend_database::providers::{
    CredentialRow, CredentialWrite, NewProvider, ProviderRow, ResealConfigs,
};
use demi_backend_page_sync::{Part, SyncRegistry};
use demi_provider_common::Secret;
use demi_provider_common::credentials::{
    CredentialPool, MemoryCredentialPool, RefreshGates, decode_secret,
};
use demi_shared_types::{Timestamp, WireApi};
use demi_web_api_protocol::auth::{Role, UserDto};
use demi_web_api_protocol::ids::{CredentialId, ProviderId, UserId};
use demi_web_api_protocol::providers::{
    ConfiguredModels, CredentialKind, ProviderDto, Rotation, SetupPart,
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
}

/// How an entry authenticates.
#[derive(Debug, Clone, PartialEq)]
pub enum EntryCredential {
    ApiKey(ApiKeyConfig),
    /// Accounts in the entry's credential records, of which `active` is the
    /// one the entry infers with unless `rotation` gives each agent one of
    /// its own (`providers.md` § Account rotation).
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
}

impl ApiKeyConfig {
    /// Whether the entry is a **compatible entry** (`providers.md` §
    /// Endpoints): it names no vendor. A vendor entry uses its vendor's
    /// endpoint and lists its vendor's catalog; a compatible entry has the
    /// endpoint the user gives, none until then, and lists only the models
    /// the user configures.
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
    /// (`models.md` § A provider is in the list from the start): a vendor
    /// entry's key, a compatible entry's endpoint (its key is optional), a
    /// model when its catalog is its configured list and the list is empty
    /// (a compatible entry's always is), and a subscription's account. A
    /// vendor entry's address is never missing: it uses its vendor's. Empty
    /// for a complete entry.
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

    /// The entry as the web app sees it: never its key. `default_endpoint` is
    /// its family's default base URL, which a vendor entry that stores no
    /// endpoint goes to; a compatible entry without one goes nowhere yet.
    pub fn dto(&self, default_endpoint: Option<EndpointUrl>) -> ProviderDto {
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
    /// the change (`claude-code.md` § Preparing a process ahead); `false`
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

    /// Whose entries `user` infers with (`product.md` § Instance mode): their
    /// own on an isolated instance, the master's on a shared one.
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

    /// The owner's entries, oldest first.
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
    /// pool, in one transaction (`providers.md` § Login and publication):
    /// the entry, its accounts, and the staged active account, or its first.
    /// `None` when the owner already holds the family's subscription entry,
    /// and then nothing is stored.
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
                secret: self.seal_secret(&id, &account, &secret),
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

    /// Replaces the entry's label, family, configuration or rotation;
    /// `None` for an entry that no longer exists.
    pub async fn update(
        &self,
        id: ProviderId,
        label: Option<String>,
        family: Option<String>,
        config: Option<ApiKeyConfig>,
        rotation: Option<Rotation>,
    ) -> Result<Option<ProviderEntry>, StorageError> {
        let sealed = config.map(|config| self.seal_config(&id, &config));
        let row = self
            .0
            .control
            .update_provider(id, label, family, sealed, rotation)
            .await?;
        if let Some(row) = &row {
            self.mark_changed(&row.owner);
        }
        row.map(|row| self.decode(row)).transpose()
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
/// configuration format 2 (`storage.md` § Schemas and migrations): it opens
/// each configuration of format 1 under the vault's key, decodes and checks
/// it as any configuration is, and seals it again in format 2. One that
/// names neither a vendor nor an endpoint was its family's own vendor's
/// entry and now names that vendor; one with an endpoint and no vendor stays
/// a compatible entry.
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

    /// A control database of the schema before provider configuration
    /// format 2, its entries sealed as 0.1.21 sealed them, upgrades as the
    /// backend opens it: the family's own vendor's entry names its vendor,
    /// and an endpoint without a vendor stays a compatible entry. One whose
    /// configuration does not open under the vault's key stops the open
    /// and still records the earlier schema.
    /// Planted defect this catches: format 1 decoded as format 2, or a
    /// family's own entry left naming no vendor, which reads as a
    /// compatible entry without an endpoint. Two small databases, a few
    /// milliseconds.
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
            // shares and the one before a device's mark had a number follow
            // in the history.
            schema_testing::store_as_shipped_before(&path, DatabaseKind::Control, 6);
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

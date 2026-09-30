//! Provider entries (`providers.md` § Credential vault, § Scope): each
//! entry's record with its configuration sealed, whose entries a user infers
//! with, and the account records of a subscription entry. The vault opens a
//! configuration when it reads an entry and validates it strictly; a value
//! that does not open or decode is corrupt and is never repaired.

use std::sync::Arc;

use demi_backend_storage::StorageError;
use demi_backend_storage::control::ControlService;
use demi_backend_storage::providers::{CredentialRow, CredentialWrite, NewProvider, ProviderRow};
use demi_backend_sync::{Part, SyncRegistry};
use demi_core::{Timestamp, WireApi};
use demi_provider::Secret;
use demi_provider::credentials::{CredentialPool, MemoryCredentialPool, RefreshGates, decode_secret};
use demi_web_api::auth::{Role, UserDto};
use demi_web_api::ids::{CredentialId, ProviderId, UserId};
use demi_web_api::providers::{ConfiguredModels, CredentialKind, ProviderDto};
use demi_web_api::settings::InstanceMode;
use demi_web_api::text::EndpointUrl;
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
}

/// How an entry authenticates.
#[derive(Debug, Clone, PartialEq)]
pub enum EntryCredential {
    ApiKey(ApiKeyConfig),
    /// Accounts in the entry's credential records, of which `active` is the
    /// one the entry infers with.
    Subscription {
        active: Option<CredentialId>,
    },
}

/// An API-key entry's configuration: the document the vault seals. A field
/// it does not name is an error, so a misspelled setting is reported rather
/// than ignored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApiKeyConfig {
    #[garde(skip)]
    pub api_key: Secret,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[garde(skip)]
    pub base_url: Option<EndpointUrl>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[garde(skip)]
    pub wire_api: Option<WireApi>,
    /// The models.dev vendor the entry was added from.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[garde(inner(length(min = 1)))]
    pub vendor_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[garde(dive)]
    pub models: Option<ConfiguredModels>,
}

impl ProviderEntry {
    pub fn kind(&self) -> CredentialKind {
        match self.credential {
            EntryCredential::ApiKey(_) => CredentialKind::ApiKey,
            EntryCredential::Subscription { .. } => CredentialKind::Subscription,
        }
    }

    /// A subscription entry's active account.
    pub fn active(&self) -> Option<&CredentialId> {
        match &self.credential {
            EntryCredential::ApiKey(_) => None,
            EntryCredential::Subscription { active } => active.as_ref(),
        }
    }

    /// The entry as the browser sees it: never its key.
    pub fn dto(&self) -> ProviderDto {
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
            base_url: config.and_then(|config| config.base_url.clone()),
            models: config.and_then(|config| config.models.clone()),
            created_at: self.created_at,
        }
    }
}

impl Vault {
    pub fn new(control: ControlService, key: VaultKey, mode: InstanceMode, sync: SyncRegistry) -> Self {
        Self(Arc::new(Shared {
            control,
            key,
            mode,
            master: tokio::sync::OnceCell::new(),
            gates: RefreshGates::new(),
            sync,
        }))
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
    pub async fn visible(&self, user: &UserId, id: &ProviderId) -> Result<Option<ProviderEntry>, StorageError> {
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
            let account = CredentialId::try_from(meta.id).map_err(|error| StorageError::Corrupt {
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
            .and_then(|active| accounts.iter().find(|account| account.id.as_str() == active))
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

    /// Replaces the entry's label or configuration; `None` for an entry that
    /// no longer exists.
    pub async fn update(
        &self,
        id: ProviderId,
        label: Option<String>,
        config: Option<ApiKeyConfig>,
    ) -> Result<Option<ProviderEntry>, StorageError> {
        let sealed = config.map(|config| self.seal_config(&id, &config));
        let row = self.0.control.update_provider(id, label, sealed).await?;
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

    /// The entry's account records, ordered by id, their secrets unopened.
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

    pub fn seal_secret(&self, provider: &ProviderId, account: &CredentialId, secret: &str) -> Vec<u8> {
        self.0.key.seal(seal::Row::Secret(provider, account), secret.as_bytes())
    }

    fn seal_config(&self, id: &ProviderId, config: &ApiKeyConfig) -> Vec<u8> {
        // A configuration is strings, numbers and lists, which always
        // serialize.
        let document = serde_json::to_vec(config).expect("an entry's configuration serializes");
        self.0.key.seal(seal::Row::Config(id), &document)
    }

    fn decode(&self, row: ProviderRow) -> Result<ProviderEntry, StorageError> {
        let credential = match (row.kind, row.config) {
            (CredentialKind::ApiKey, Some(sealed)) => EntryCredential::ApiKey(self.open_config(&row.id, &sealed)?),
            (CredentialKind::Subscription, None) => EntryCredential::Subscription { active: row.active },
            // The schema ties the configuration to the kind.
            _ => return Err(corrupt_config("the configuration does not match the entry's kind")),
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

    /// The configuration sealed for entry `id`: it must open, be UTF-8 JSON
    /// of the configuration's shape, and keep its rules. A failure names the
    /// field and the kind of fault, never a value, which may be the key.
    fn open_config(&self, id: &ProviderId, sealed: &[u8]) -> Result<ApiKeyConfig, StorageError> {
        let document = self
            .0
            .key
            .open(seal::Row::Config(id), sealed)
            .map_err(|error| corrupt_config(error.to_string()))?;
        let text = String::from_utf8(document).map_err(|_| corrupt_config("the configuration is not UTF-8"))?;
        let config: ApiKeyConfig = decode_secret(&text).map_err(|error| corrupt_config(error.to_string()))?;
        config.validate().map_err(|report| corrupt_config(report.to_string()))?;
        Ok(config)
    }
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

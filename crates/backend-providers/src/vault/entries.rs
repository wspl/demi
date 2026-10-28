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
//! every entry belongs to a user. Instance mode, which the master changes at
//! any time, only decides whose entries a user infers with: a change moves no
//! entry, and each reader of the entries meets the new owner's from its next
//! read. There is no ownerless entry and no second kind of credential.
//!
//! A shared instance offers no subscription entry: the vault reads none, the
//! master's included, and refuses a sign-in. The entries stay stored for the
//! isolated mode.
//!
//! On a shared instance the master's credentials are used by many users on
//! many workers at once, so:
//!
//! - Refreshes rely on the versioned replace of the credential pool.
//! - The vendor's windows describe the account and not a user. Per-user
//!   consumption is the usage ledger's.
//! - A Claude Code sign-in stays on the Claude Code machine.

use std::sync::Arc;

use demi_backend_database::StorageError;
use demi_backend_database::control::ControlService;
use demi_backend_database::providers::{
    CredentialRow, CredentialWrite, MediaSwitches, NewProvider, ProviderRow, ResealSealed,
};
use demi_backend_page_sync::{Part, SyncRegistry};
use demi_provider_common::credentials::{
    AccountMeta, CredentialPool, MemoryCredentialPool, NewAccount, RefreshGates,
    credential_id_for, decode_secret,
};
use demi_shared_types::{CacheTtl, Timestamp};
use demi_web_api_protocol::auth::{Role, UserDto};
use demi_web_api_protocol::ids::{CredentialId, ProviderId, UserId};
use demi_web_api_protocol::providers::{CredentialKind, Rotation};
use demi_web_api_protocol::settings::InstanceMode;
use garde::Validate;
use serde::{Deserialize, Serialize};

use demi_provider_registry::entry::{ApiKeyConfig, EntryCredential, ProviderEntry};

use super::pool::VaultCredentialPool;
use super::seal::{self, VaultKey};

/// The credential vault. Cloning it is cheap.
#[derive(Clone)]
pub struct Vault(Arc<Shared>);

struct Shared {
    control: ControlService,
    key: VaultKey,
    /// The mode the control store holds, which `set_mode` writes after it.
    mode: std::sync::RwLock<InstanceMode>,
    shared_subscriptions: bool,
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
/// misses some: a broadcast channel holds its backlog in memory, so it has a
/// bound, and the changes come from the user's own clicks in Settings.
const ACCOUNT_CHANGES_KEPT: usize = 64;

/// An account a subscription entry is created with outside a sign-in:
///
/// - The development backend's Codex sign-in, which the master account's
///   entry holds from each start (`crates/xtask/src/dev/mod.rs`).
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


/// The format of the sealed configuration document this release writes
/// and reads, which the control schema names. Format 1, the bare
/// configuration that 0.1.21 and the builds before this one sealed, told a
/// compatible entry by its endpoint rather than by its vendor; the control
/// database's migration re-encodes it ([`VaultReseal`]).
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


impl Vault {
    /// The vault of an instance whose control store holds `mode`.
    pub fn new(
        control: ControlService,
        key: VaultKey,
        mode: InstanceMode,
        shared_subscriptions: bool,
        sync: SyncRegistry,
    ) -> Self {
        Self(Arc::new(Shared {
            control,
            key,
            mode: std::sync::RwLock::new(mode),
            shared_subscriptions,
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

    pub fn mode(&self) -> InstanceMode {
        *self.0.mode.read().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Changes the mode, and has every page read the entries, the models and
    /// the mode again.
    pub async fn set_mode(&self, mode: InstanceMode) -> Result<(), StorageError> {
        self.0.control.set_instance_mode(mode).await?;
        *self.0.mode.write().unwrap_or_else(std::sync::PoisonError::into_inner) = mode;
        for part in [Part::Instance, Part::Providers, Part::Models] {
            self.0.sync.mark_everyone(&part);
        }
        Ok(())
    }

    /// Whether the instance offers subscription entries now: an isolated
    /// one does.
    pub fn offers_subscriptions(&self) -> bool {
        self.mode() == InstanceMode::Isolated || self.0.shared_subscriptions
    }

    pub fn shared_subscriptions(&self) -> bool {
        self.0.shared_subscriptions
    }

    /// Whether `user` configures the entries of their scope: everyone on an
    /// isolated instance, only the master on a shared one.
    pub fn configures(&self, user: &UserDto) -> bool {
        self.mode() == InstanceMode::Isolated || user.role == Role::Master
    }

    /// Whose entries `user` infers with: their own on an isolated instance, the
    /// master's on a shared one.
    pub async fn owner_for(&self, user: &UserId) -> Result<UserId, StorageError> {
        if self.mode() == InstanceMode::Isolated {
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
        match self.mode() {
            InstanceMode::Shared => self.0.sync.mark_everyone(&Part::Providers),
            InstanceMode::Isolated => self.0.sync.mark(owner, Part::Providers),
        }
    }

    /// Marks the providers changed for every user who infers with the entry
    /// `id`. A lookup that fails is logged: the pages then show the change
    /// with the entry's next one.
    pub async fn mark_entry_changed(&self, id: &ProviderId) {
        if self.mode() == InstanceMode::Shared {
            self.0.sync.mark_everyone(&Part::Providers);
            return;
        }
        match self.0.control.provider(id.clone()).await {
            Ok(Some(row)) => self.0.sync.mark(&row.owner, Part::Providers),
            // An entry deleted meanwhile marked its owner as it went.
            Ok(None) => {}
            Err(error) => tracing::warn!(
                event = "entry_change.announce_failed",
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

    /// The owner's entries the instance offers, in the order of Settings ›
    /// Providers.
    pub async fn entries(&self, owner: UserId) -> Result<Vec<ProviderEntry>, StorageError> {
        let rows = self.0.control.providers(owner).await?;
        let offered = self.offers_subscriptions();
        rows.into_iter()
            .map(|row| self.decode(row))
            .filter(|entry| !entry.as_ref().is_ok_and(|entry| !offered && entry.kind() == CredentialKind::Subscription))
            .collect()
    }

    /// The entry `id`, when the instance offers it.
    pub async fn entry(&self, id: ProviderId) -> Result<Option<ProviderEntry>, StorageError> {
        let row = self.0.control.provider(id).await?;
        let entry = row.map(|row| self.decode(row)).transpose()?;
        Ok(entry.filter(|entry| self.offers_subscriptions() || entry.kind() != CredentialKind::Subscription))
    }

    /// Stores a new API-key entry.
    pub async fn create_api_key(
        &self,
        owner: UserId,
        family: String,
        label: String,
        config: ApiKeyConfig,
        cache_ttl: Option<CacheTtl>,
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
            cache_ttl,
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
            cache_ttl: None,
        };
        let row = self.0.control.insert_provider(provider, accounts).await?;
        if let Some(row) = &row {
            self.mark_changed(&row.owner);
        }
        row.map(|row| self.decode(row)).transpose()
    }

    /// Makes the owner's entry of the family hold `seeded`'s account, as a
    /// sign-in would have published it.
    ///
    /// - With no such entry, creates it with the account as its only one.
    /// - With one, stores the account in it and makes it the active one, so
    ///   a seed an earlier start made, whose access token has since expired,
    ///   is replaced by the current one. An account of the same identity
    ///   that came from another source, such as a sign-in that holds a
    ///   refresh token, is left as it is.
    pub async fn seed(
        &self,
        owner: UserId,
        seeded: &SeededAccount,
        now: Timestamp,
    ) -> Result<(), StorageError> {
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
        staged
            .write(meta.clone(), secret.clone())
            .await
            .expect("a pool in memory stores");
        staged.set_active(&id).await.expect("a pool in memory selects");
        if self
            .create_subscription(owner.clone(), seeded.family.clone(), seeded.label.clone(), &staged)
            .await?
            .is_some()
        {
            return Ok(());
        }
        let held = self.entries(owner).await?.into_iter().find(|entry| {
            entry.family == seeded.family && entry.kind() == CredentialKind::Subscription
        });
        let Some(entry) = held else {
            return Ok(());
        };
        let id = CredentialId::try_from(id).map_err(|error| StorageError::Corrupt {
            table: "provider_credentials",
            column: "id",
            reason: format!("a seeded account's id is invalid: {error}"),
        })?;
        if let Some(existing) = self.account(entry.id.clone(), id.clone()).await? {
            if existing.source != seeded.source {
                return Ok(());
            }
        }
        let account = CredentialWrite {
            secret: secret.map(|secret| self.seal_secret(&entry.id, &id, &secret)),
            id: id.clone(),
            identity_key: meta.identity_key,
            label: meta.label,
            detail: meta.detail,
            source: meta.source,
        };
        self.write_account(&entry.id, account).await?;
        if entry.active() != Some(&id) {
            self.set_active_account(&entry.id, id).await?;
        }
        self.mark_changed(&entry.owner);
        Ok(())
    }

    /// Replaces the entry's label, family, configuration, rotation, media
    /// switches or prompt cache lifetime; `None` for an entry that no longer
    /// exists.
    #[allow(clippy::too_many_arguments)]
    pub async fn update(
        &self,
        id: ProviderId,
        label: Option<String>,
        family: Option<String>,
        config: Option<ApiKeyConfig>,
        rotation: Option<Rotation>,
        switches: MediaSwitches,
        cache_ttl: Option<Option<CacheTtl>>,
    ) -> Result<Option<ProviderEntry>, StorageError> {
        let sealed = config.map(|config| self.seal_config(&id, &config));
        let row = self
            .0
            .control
            .update_provider(id, label, family, sealed, rotation, switches, cache_ttl)
            .await?;
        if let Some(row) = &row {
            self.mark_changed(&row.owner);
        }
        row.map(|row| self.decode(row)).transpose()
    }

    /// Puts the owner's entries in the order `ids` names them; `false`,
    /// changing nothing, unless it names each one the instance offers once.
    /// The entries it does not offer, which no page lists, keep their order
    /// before the others, where the subscriptions are.
    pub async fn set_order(&self, owner: &UserId, ids: Vec<ProviderId>) -> Result<bool, StorageError> {
        let mut all = Vec::new();
        if !self.offers_subscriptions() {
            for row in self.0.control.providers(owner.clone()).await? {
                let entry = self.decode(row)?;
                if entry.kind() == CredentialKind::Subscription {
                    all.push(entry.id);
                }
            }
        }
        all.extend(ids);
        let ordered = self.0.control.set_provider_order(owner.clone(), all).await?;
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
            cache_ttl: row.cache_ttl,
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

/// The vault's side of the control database's migrations that seal a value
/// again (`crates/backend-database/src/schema.rs`).
///
/// - To provider configuration format 2, it opens each configuration of format
///   1 under the vault's key, decodes and checks it as any configuration is,
///   and seals it again in format 2. A configuration that names neither a
///   vendor nor an endpoint was its family's own vendor's entry and now names
///   that vendor; one with an endpoint and no vendor stays a compatible entry.
/// - To schedules, it seals a URL secret as a schedule's in place of an
///   automation's.
pub struct VaultReseal(VaultKey);

impl VaultReseal {
    pub fn new(key: VaultKey) -> Self {
        Self(key)
    }
}

impl ResealSealed for VaultReseal {
    fn provider_config(&self, id: &ProviderId, family: &str, sealed: &[u8]) -> Result<Vec<u8>, String> {
        let text = open_text(&self.0, id, sealed)?;
        let config = decode_secret::<ApiKeyConfig>(&text).map_err(|error| error.to_string())?;
        let mut config = checked(config)?;
        if config.vendor_id.is_none() && config.base_url.is_none() {
            config.vendor_id = Some(family.to_owned());
        }
        Ok(seal_config(&self.0, id, &config))
    }

    fn schedule_secret(&self, id: &str, sealed: &[u8]) -> Result<Vec<u8>, String> {
        let secret = self
            .0
            .open(seal::Row::AutomationSecret(id), sealed)
            .map_err(|error| error.to_string())?;
        Ok(self.0.seal(seal::Row::ScheduleSecret(id), &secret))
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

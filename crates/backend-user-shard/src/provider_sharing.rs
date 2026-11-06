//! Lending an entry to another server (`crates/provider-relay/src/lib.rs`):
//! the entry's share keys, and the sessions borrowed with them.
//!
//! - Whoever manages an entry makes and revokes its keys. A key works while
//!   its maker still manages the entry, and a borrowed entry, which this
//!   server never lends on, has none.
//! - A key is given once, in the answer that makes it: the store keeps its
//!   SHA-256, as it keeps a browser's session token.
//! - A key never used stops working [`UNUSED_FOR_MS`] after it is made; a
//!   used one works until it is revoked.
//! - Each use marks the key with the server that used it, by the host it
//!   gives, and when. Nothing else of a borrowed session is kept: its usage
//!   is the borrower's.
//! - Revoking a key closes every session it opened and ends every media call
//!   it made, which cancels a video's job left unfinished
//!   (`demi_core::shard::lent_media`).
//!
//! A borrowed session runs on the shard of the key's maker, on the runtime a
//! conversation of theirs would have, unmetered
//! (`crates/backend-user-shard/src/conversation/providers.rs`).

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Mutex;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use demi_core_store::StorageError;
use demi_backend_database::accounts::TokenHash;
use demi_backend_database::provider_keys::ShareKeyRow;
use demi_core_providers::vault::entries::Vault;
use demi_provider_registry::entry::ProviderEntry;
use demi_provider_relay::borrow::FAMILY as BORROWED;
use demi_provider_relay::key::ShareKey as KeyText;
use demi_provider_relay::lend::Lends;
use demi_provider_relay::wire::{ClientMessage, Lent, ModelLimits, ServerMessage};
use demi_shared_types::{ProviderModelList, Timestamp};
use demi_web_api_protocol::ids::UserId;
use demi_web_api_protocol::providers::{NewShareKey, ShareKey, ShareKeyUse, ShareKeys};
use futures_util::{Sink, Stream};
use tokio_util::sync::CancellationToken;

use demi_core::conversation::{lent_limits, lent_runtime};
use demi_core::provider_settings::ProviderRefusal;
use crate::server_starter::ServerCore;
use crate::services::Services;
use demi_core::shard::Shard;

/// How long a key nobody used keeps working: a key pasted into a chat and
/// forgotten should not stay valid, and a week is how long an invite link
/// lasts by default in the apps people share them from.
pub const UNUSED_FOR_MS: i64 = 7 * 24 * 60 * 60 * 1000;

/// The open borrowed sessions and media calls of each key, which revoking it
/// ends.
#[derive(Default)]
pub struct LentSessions(Mutex<HashMap<String, Vec<CancellationToken>>>);

impl LentSessions {
    pub(crate) fn open(&self, key: &str) -> CancellationToken {
        let token = CancellationToken::new();
        let mut sessions = self.0.lock().expect("the lent sessions' lock is never poisoned");
        sessions.entry(key.to_owned()).or_default().push(token.clone());
        token
    }

    /// Forgets the session or call of `token`, which ended.
    pub(crate) fn ended(&self, key: &str, token: &CancellationToken) {
        token.cancel();
        let mut sessions = self.0.lock().expect("the lent sessions' lock is never poisoned");
        if let Some(open) = sessions.get_mut(key) {
            open.retain(|open| !open.is_cancelled());
            if open.is_empty() {
                sessions.remove(key);
            }
        }
    }

    fn revoke(&self, key: &str) {
        let removed = self.0.lock().expect("the lent sessions' lock is never poisoned").remove(key);
        for token in removed.into_iter().flatten() {
            token.cancel();
        }
    }
}

/// A key a borrowing server presented, and the entry it lends.
pub struct Borrowing {
    pub key: ShareKeyRow,
    pub entry: ProviderEntry,
}

impl Services {
    /// When the keys never used and made before it stopped working.
    fn unused_since(&self) -> Timestamp {
        let now = self.clock.now();
        Timestamp::from_millisecond(now.as_millisecond() - UNUSED_FOR_MS).unwrap_or(now)
    }

    /// The entry `id` when `user` manages it and it can be lent.
    async fn lendable(&self, user: &UserId, id: &str) -> Result<ProviderEntry, ProviderRefusal> {
        let entry = self.managed_entry(user, id).await?;
        if entry.family == BORROWED {
            return Err(ProviderRefusal::Borrowed(entry.label));
        }
        Ok(entry)
    }

    /// The share keys of the entry `id`, newest first.
    pub async fn share_keys(&self, user: &UserId, id: &str) -> Result<ShareKeys, ProviderRefusal> {
        let entry = self.lendable(user, id).await?;
        let rows = self.control.share_keys(entry.owner, entry.id, self.unused_since()).await?;
        Ok(ShareKeys {
            keys: rows.into_iter().map(shown).collect(),
        })
    }

    /// Makes a share key of the entry `id`, which the answer alone holds.
    pub async fn new_share_key(&self, user: &UserId, id: &str) -> Result<NewShareKey, ProviderRefusal> {
        let entry = self.lendable(user, id).await?;
        let url = self
            .public_url
            .get()
            .ok_or_else(|| ProviderRefusal::Unlendable("This server has no address yet".into()))?
            .url()
            .clone();
        let token = URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>());
        let row = ShareKeyRow {
            id: URL_SAFE_NO_PAD.encode(rand::random::<[u8; 12]>()),
            owner: entry.owner.clone(),
            provider: entry.id.clone(),
            created_by: user.clone(),
            created_at: self.clock.now(),
            used: None,
        };
        self.control.insert_share_key(TokenHash::of(&token), row.clone()).await?;
        Ok(NewShareKey {
            key: shown(row),
            text: KeyText { url, token }.text(),
        })
    }

    /// Revokes the entry `id`'s share key `key`, and closes the sessions
    /// borrowed with it.
    pub async fn revoke_share_key(&self, user: &UserId, id: &str, key: &str) -> Result<(), ProviderRefusal> {
        let entry = self.lendable(user, id).await?;
        if !self.control.delete_share_key(entry.owner, entry.id, key.to_owned()).await? {
            return Err(ProviderRefusal::NotFound);
        }
        self.end_lending(&[key.to_owned()]);
        Ok(())
    }

    /// The ids of the entry `entry`'s share keys, whose sessions
    /// [`end_lending`](Self::end_lending) closes once deleting the entry
    /// deleted them.
    pub(crate) async fn lent_keys(&self, entry: &ProviderEntry) -> Result<Vec<String>, StorageError> {
        let rows = self.control.share_keys(entry.owner.clone(), entry.id.clone(), self.unused_since()).await?;
        Ok(rows.into_iter().map(|row| row.id).collect())
    }

    /// Closes the sessions borrowed with `keys`, and ends their media calls.
    pub(crate) fn end_lending(&self, keys: &[String]) {
        for key in keys {
            self.lent.revoke(key);
        }
    }

    /// What the share key `token` lends, marked as used by `server`; none
    /// for a key that is unknown, revoked or expired, or whose maker no
    /// longer manages its entry.
    pub async fn borrowing(&self, token: &str, server: String) -> Result<Option<Borrowing>, StorageError> {
        let now = self.clock.now();
        let Some(key) = self
            .control
            .use_share_key(TokenHash::of(token), server, now, self.unused_since())
            .await?
        else {
            return Ok(None);
        };
        let entry = self.vault.visible(&key.created_by, &key.provider).await?;
        Ok(entry
            .filter(|entry| Vault::manages(&key.created_by, entry) && entry.family != BORROWED)
            .map(|entry| Borrowing { key, entry }))
    }
}

/// A key as Share Provider shows it.
fn shown(row: ShareKeyRow) -> ShareKey {
    let expires_at = match row.used {
        Some(_) => None,
        None => Timestamp::from_millisecond(row.created_at.as_millisecond() + UNUSED_FOR_MS).ok(),
    };
    ShareKey {
        id: row.id,
        created_at: row.created_at,
        used: row.used.map(|(server, at)| ShareKeyUse { server, at }),
        expires_at,
    }
}

impl Services {
    /// Whether the key's maker still lends `borrowing`'s entry: one they
    /// turned off in Settings › Providers lends nothing until it is on.
    pub async fn lends(&self, borrowing: &Borrowing) -> Result<bool, StorageError> {
        Ok(self.lends_model(borrowing, None).await? == Lends::Yes)
    }

    /// Whether the key's maker lends `borrowing`'s entry and, when given,
    /// its `model`: they lend only what they have on in Settings ›
    /// Providers.
    async fn lends_model(&self, borrowing: &Borrowing, model: Option<&str>) -> Result<Lends, StorageError> {
        let preferences = self.stores.of(&borrowing.key.created_by).await?.preferences().await?;
        let entry = &borrowing.entry.id;
        Ok(if preferences.hides_provider(entry) {
            Lends::TurnedOff
        } else if model.is_some_and(|model| preferences.hides_model(entry, model)) {
            Lends::NotTheModel
        } else {
            Lends::Yes
        })
    }
}

/// The user's provider entries lent to other servers, on the user's core.
#[expect(
    async_fn_in_trait,
    reason = "a core runs on one thread: its futures are never sent to another"
)]
pub trait ProviderSharing {
    /// What the lender says of `borrowing`'s entry: its name, its catalog of
    /// the models the key's maker has on, and what one request of each may
    /// hold; nothing but its name while they have it turned off.
    async fn lent(self: Rc<Self>, borrowing: Borrowing) -> Result<Lent, String>;

    /// Serves the session borrowed with `borrowing`'s key over a socket's
    /// messages until it closes or the key is revoked.
    async fn lend<In, Out>(self: Rc<Self>, borrowing: Borrowing, incoming: In, outgoing: Out)
    where
        In: Stream<Item = ClientMessage> + Unpin,
        Out: Sink<ServerMessage> + Unpin,
;
}

impl ProviderSharing for Shard {
    async fn lent(self: Rc<Self>, borrowing: Borrowing) -> Result<Lent, String> {
        let services = self.server_services().clone();
        let registry = &services.provider_registry;
        let entry = &borrowing.entry;
        let preferences = self
            .server_services()
            .stores
            .of(&borrowing.key.created_by)
            .await
            .map_err(|error| error.to_string())?
            .preferences()
            .await
            .map_err(|error| error.to_string())?;
        if preferences.hides_provider(&entry.id) {
            return Ok(Lent {
                name: entry.label.clone(),
                catalog: ProviderModelList {
                    models: Vec::new(),
                    default_model_id: None,
                    warnings: Vec::new(),
                    source_fetched_at: services.clock.now(),
                    stale: false,
                },
                limits: Vec::new(),
                turned_off: true,
            });
        }
        let provider = registry.provider_for(entry).await;
        let mut catalog = registry.entry_catalog(entry, &provider, false).await;
        catalog.models.retain(|model| !preferences.hides_model(&entry.id, &model.id));
        if catalog.default_model_id.as_ref().is_some_and(|id| !catalog.models.iter().any(|model| &model.id == id)) {
            catalog.default_model_id = None;
        }
        // A warning is the source's own words, such as an account signed
        // out, which stay here; the borrower hears only that there is one.
        if !catalog.warnings.is_empty() {
            for warning in &catalog.warnings {
                tracing::info!(event = "relay.catalog_warning", key = %borrowing.key.id, warning, "a lent entry's catalog has a warning");
            }
            catalog.warnings = vec!["The provider could not list its models.".to_owned()];
        }
        let selections: Vec<_> = catalog
            .models
            .iter()
            .map(|model| model.selection(entry.id.as_str(), None, None))
            .collect();
        let limits = lent_limits(
            &self,
            services.core.clone(),
            borrowing.key.created_by.clone(),
            entry,
            &selections,
            borrowing.key.id.clone(),
        )
        .await?;
        Ok(Lent {
            name: entry.label.clone(),
            turned_off: false,
            catalog,
            limits: limits
                .into_iter()
                .map(|(model, limits)| ModelLimits { model, limits })
                .collect(),
        })
    }

    async fn lend<In, Out>(self: Rc<Self>, borrowing: Borrowing, incoming: In, outgoing: Out)
    where
        In: Stream<Item = ClientMessage> + Unpin,
        Out: Sink<ServerMessage> + Unpin,
    {
        let services = self.server_services().clone();
        let key = borrowing.key.id.clone();
        let revoked = services.lent.open(&key);
        // A revoke, or the entry's deletion, between the key's check and
        // `open` found no session to close: the key is checked again now
        // that one would be found.
        let held = services.lent_keys(&borrowing.entry).await;
        if !held.is_ok_and(|keys| keys.contains(&key)) {
            services.lent.ended(&key, &revoked);
            return;
        }
        let runtime = lent_runtime(
            &self,
            services.core.clone(),
            borrowing.key.created_by.clone(),
            borrowing.entry.id.clone(),
            key.clone(),
        );
        let lends = async |model: &str| services.lends_model(&borrowing, Some(model)).await.map_err(|error| error.to_string());
        tokio::select! {
            () = demi_provider_relay::lend::serve(runtime, incoming, outgoing, &key, lends) => {}
            () = revoked.cancelled() => {}
        }
        services.lent.ended(&key, &revoked);
    }
}

//! The server's parts of the product state and of a page's synchronization
//! channel, beside the core's (`crates/core-api/src/state.rs`): the
//! signed-in user, the passkeys, the sessions, the accounts, the Cloud and its
//! isolation, the server's updates, and the installers' runner release.
//!
//! An account still to choose its password sees its account alone, until it
//! is chosen; an account made an administrator starts to see the accounts
//! and the server's updates.

use std::time::Instant;

use demi_backend_accounts::passkeys::{passkey_list, relying_party_id};
use demi_backend_cloud::user_core::CloudOfCore;
use demi_backend_database::accounts::TokenHash;
use demi_backend_runners::install::install_commands;
use demi_core::Core;
use demi_core_api::caller::CallerPage;
use demi_core_api::state::{StateError, core_state, millis, timed};
use demi_core_store::StorageError;
use demi_core_sync::{Part, etag_of};
use demi_web_api_protocol::auth::{Role, UserDto};
use demi_web_api_protocol::ids::ConversationId;
use demi_web_api_protocol::sessions::SessionId;
use demi_web_api_protocol::state::{ProductState, SyncEvent};
use demi_web_api_protocol::users::Users;
use futures_util::future::LocalBoxFuture;

use crate::page_notifications::OpenPage;
use crate::server_starter::{ServerCore, ServerSide};

/// The product state `user`'s page reads, on the user's core: the core's,
/// with the server's parts.
pub async fn product_state(core: &Core, user: UserDto) -> Result<ProductState, StateError> {
    let services = core.server_services();
    let core_part = core_state(core).await?;
    let started = Instant::now();
    let (mut cloud_ms, mut release_ms) = (0, 0);
    let cloud = timed(&mut cloud_ms, core.cloud_core().cloud_status()).await?;
    let runner_release = match timed(&mut release_ms, services.runner_releases.current()).await {
        Ok(release) => release.map(|release| release.release),
        Err(error) => {
            // The installers and the runners' updates report the deployment's
            // broken record; the pages show the runners' releases as unknown
            // meanwhile.
            tracing::warn!(event = "runner_release.read_failed", %error, "the current runner release cannot be read");
            None
        }
    };
    tracing::info!(
        event = "state.server_parts_read",
        ms = millis(started),
        cloud_ms,
        runner_release_ms = release_ms,
        "the server's parts of the product state were read"
    );
    let public_url = services
        .public_url
        .get()
        .expect("the backend listens before it serves a request")
        .url();
    let server_upgrade = if user.role == Role::User { None } else { services.upgrades.snapshot() };
    Ok(ProductState {
        core: core_part,
        user,
        public_url: public_url.as_str().to_owned(),
        install_commands: install_commands(public_url),
        runner_release,
        cloud,
        passkey_rp_id: relying_party_id(public_url).map(str::to_owned),
        web_build: services.web_build.clone(),
        mail: services.email.delivers(),
        server_upgrade,
    })
}

/// A page of a signed-in session, as its synchronization channel serves it
/// on the user's core.
pub struct SessionPage {
    /// The session's user as the page last learned it.
    user: UserDto,
    /// The session's token, which tells this session from the user's
    /// others in the sessions' list.
    token: TokenHash,
    /// What the page shows and when it was used, for the session's
    /// notifications.
    open: OpenPage,
}

impl SessionPage {
    /// The page of the session `session`, whose token is `token`, of `user`,
    /// on `core`.
    pub fn new(core: &Core, session: SessionId, token: TokenHash, user: UserDto) -> Self {
        Self {
            user,
            token,
            open: ServerSide::of(core).pages.page(session),
        }
    }
}

impl CallerPage for SessionPage {
    fn only(&self) -> Option<Part> {
        // An account still to choose its password sees the account alone,
        // until it is chosen, on this page or another.
        self.user.chooses_password.then_some(Part::User)
    }

    fn read<'a>(&'a self, core: &'a Core, part: &'a Part) -> LocalBoxFuture<'a, Result<Option<SyncEvent>, StateError>> {
        Box::pin(read_server_part(core, part, &self.user, &self.token))
    }

    fn sent(&mut self, event: &SyncEvent) -> Vec<Part> {
        let SyncEvent::User { user } = event else {
            return Vec::new();
        };
        // An account made an administrator starts to see the accounts and the
        // server's updates.
        let promoted = self.user.role == Role::User && user.role != Role::User;
        self.user = user.clone();
        if promoted { vec![Part::Users, Part::ServerUpgrade] } else { Vec::new() }
    }

    fn show(&self, conversation: Option<ConversationId>, front: bool) {
        self.open.show(conversation, front);
    }

    fn used(&self) {
        self.open.used();
    }
}

/// The server's part `part` as the message that carries it, read now for
/// `user`, the core's user, and the page of the session `reader`; none when
/// the page is sent nothing of it.
async fn read_server_part(
    core: &Core,
    part: &Part,
    user: &UserDto,
    reader: &TokenHash,
) -> Result<Option<SyncEvent>, StateError> {
    let services = core.server_services();
    let control = &services.control;
    let event = match part {
        Part::User => {
            let Some(account) = control.account(core.user().clone()).await? else {
                return Ok(None);
            };
            SyncEvent::User { user: account.user }
        }
        Part::Passkeys => {
            let list = passkey_list(control, core.user()).await?;
            SyncEvent::Passkeys {
                etag: etag_of(&list),
                passkeys: list.passkeys,
                user_handle: list.user_handle,
            }
        }
        // The sessions' route sends no tag.
        Part::Sessions => SyncEvent::Sessions {
            sessions: services.sessions.list(core.user(), reader).await?.sessions,
            etag: String::new(),
        },
        Part::Users => {
            // A user's channel never carries the accounts.
            if user.role == Role::User {
                return Ok(None);
            }
            let list = Users {
                users: control.users().await?,
                cloud_storage_limits: demi_backend_cloud::isolation::storage_limits(control, &services.cloud).await?,
            };
            SyncEvent::Users {
                etag: etag_of(&list),
                users: list.users,
                cloud_storage_limits: list.cloud_storage_limits,
            }
        }
        Part::CloudIsolation => {
            // Only the master chooses the isolation.
            if user.role != Role::Master {
                return Ok(None);
            }
            SyncEvent::CloudIsolation {
                isolation: demi_backend_cloud::isolation::setting(control, &services.cloud).await?,
                etag: String::new(),
            }
        }
        Part::ServerUpgrade => {
            // A user's channel never carries the server's updates.
            if user.role == Role::User {
                return Ok(None);
            }
            let Some(upgrade) = services.upgrades.snapshot() else {
                return Ok(None);
            };
            SyncEvent::ServerUpgrade { upgrade }
        }
        Part::Update => SyncEvent::Update {
            state: services.upgrades.update_state(),
        },
        Part::Cloud => SyncEvent::Cloud {
            cloud: core.cloud_core().cloud_status().await.map_err(StorageError::from)?,
        },
        // The core reads its own parts.
        _ => return Ok(None),
    };
    Ok(Some(event))
}

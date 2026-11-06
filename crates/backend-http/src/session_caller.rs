//! A signed-in session as the core's routes see their caller
//! (`crates/core-api/src/caller.rs`): its user's core, the File view's grants
//! kept under its token, its verification, the end of its page connection
//! as it is signed out or expires, and the server's parts of its page's
//! product state.

use std::sync::Arc;
use std::time::Duration;

use demi_backend_database::accounts::{ResolvedSession, TokenHash};
use demi_backend_user_shard::page_state::SessionPage;
use demi_backend_user_shard::services::Services;
use demi_core::Core;
use demi_core_api::caller::{Caller, CallerPage, Gone};
use demi_core_api::error::ApiError;
use demi_web_api_protocol::connection::LaneTarget;
use demi_web_api_protocol::ids::UserId;
use futures_util::future::BoxFuture;

use super::sessions::require_verification;

/// A live session of a signed-in user, as its request found it.
pub(crate) struct SessionCaller {
    services: Arc<Services>,
    token: TokenHash,
    session: ResolvedSession,
}

impl SessionCaller {
    pub(crate) fn new(services: Arc<Services>, token: TokenHash, session: ResolvedSession) -> Self {
        Self { services, token, session }
    }
}

impl Caller for SessionCaller {
    fn principal(&self) -> &UserId {
        &self.session.user.id
    }

    fn key(&self) -> &str {
        self.token.as_str()
    }

    fn present(&self) -> BoxFuture<'_, Result<bool, ApiError>> {
        Box::pin(async move { Ok(self.services.sessions.check(&self.token).await?.is_some()) })
    }

    fn proved_lately(&self) -> Result<(), ApiError> {
        require_verification(&self.services, self.session.verified_at)
    }

    fn opens(&self, target: &LaneTarget) -> BoxFuture<'_, Result<Arc<dyn Caller>, ApiError>> {
        let opens_sync = *target == LaneTarget::Sync;
        Box::pin(async move {
            let session = self
                .services
                .sessions
                .check(&self.token)
                .await?
                .ok_or_else(ApiError::unauthenticated)?;
            if session.user.chooses_password && !opens_sync {
                return Err(ApiError::password_choice_required());
            }
            let now: Arc<dyn Caller> = Arc::new(Self::new(self.services.clone(), self.token.clone(), session));
            Ok(now)
        })
    }

    fn gone(&self) -> BoxFuture<'static, Gone> {
        let services = self.services.clone();
        let token = self.token.clone();
        let mut expires_at = self.session.expires_at;
        let watch = services.sync.watch_session(&self.session.user.id, token.clone());
        Box::pin(async move {
            loop {
                let left = expires_at.to_jiff().duration_since(services.clock.now().to_jiff());
                // An expiry already past is due at once.
                let expiry = tokio::time::sleep(Duration::try_from(left).unwrap_or(Duration::ZERO));
                tokio::select! {
                    () = watch.marked() => {
                        if watch.take().session_ended {
                            return Gone::Ended;
                        }
                        continue;
                    }
                    () = expiry => {}
                }
                // At the expiry it holds, the session is looked at again,
                // since a request may have renewed it.
                match services.sessions.check(&token).await {
                    Ok(Some(renewed)) if renewed.expires_at > expires_at => expires_at = renewed.expires_at,
                    // A session the clock has not yet passed is checked again
                    // at the expiry it holds.
                    Ok(Some(_)) => {}
                    Ok(None) => return Gone::Ended,
                    Err(error) => {
                        tracing::error!(
                            event = "page_connection.session_read_failed",
                            error = &error as &dyn std::error::Error,
                            "a page connection's session could not be read"
                        );
                        return Gone::Unread;
                    }
                }
            }
        })
    }

    fn page(&self, core: &Core) -> Box<dyn CallerPage> {
        Box::new(SessionPage::new(
            core,
            self.session.id.clone(),
            self.token.clone(),
            self.session.user.clone(),
        ))
    }
}

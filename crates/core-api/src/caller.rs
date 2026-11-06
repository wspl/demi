//! Who calls a core's routes, as whoever mounts them resolved the request.
//!
//! The server resolves the session its cookie names; a program's process,
//! the program that opened the core. The mount puts the caller in each
//! request's extensions before a route of this crate sees it, and a route
//! reaches only the core of the caller's principal.
//!
//! What only a server's caller has, a session that ends, an account still
//! to choose its password, the account's own parts of the product state,
//! is the caller's to answer; a program's caller has none of it.

use std::sync::Arc;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use demi_core::Core;
use demi_core_sync::Part;
use demi_web_api_protocol::connection::LaneTarget;
use demi_web_api_protocol::ids::{ConversationId, UserId};
use demi_web_api_protocol::state::SyncEvent;
use futures_util::future::{BoxFuture, LocalBoxFuture};

use crate::error::ApiError;
use crate::starter_lane::StarterLane;
use crate::state::StateError;

/// Who calls a core's routes.
pub trait Caller: Send + Sync + 'static {
    /// The principal of the core the caller reaches.
    fn principal(&self) -> &UserId;

    /// What the caller's every request carries and no other caller's does:
    /// the File view keeps its grants of HTML files under it.
    fn key(&self) -> &str;

    /// Whether the caller is still there, asked for a request that carries
    /// only a grant the caller was given, as the File view's.
    fn present(&self) -> BoxFuture<'_, Result<bool, ApiError>>;

    /// Refuses an act the caller must have proved itself for lately, such
    /// as revealing a secret's value.
    fn proved_lately(&self) -> Result<(), ApiError>;

    /// The caller as it is now, which each lane's open is checked with:
    /// refused once the caller is gone, or for a lane it may not open now.
    fn opens(&self, target: &LaneTarget) -> BoxFuture<'_, Result<Arc<dyn Caller>, ApiError>>;

    /// Resolves once the caller is gone, which ends its page connection;
    /// never, for a caller that stays.
    fn gone(&self) -> BoxFuture<'static, Gone>;

    /// The caller's page as its synchronization channel serves it, on the
    /// caller's core.
    fn page(&self, core: &Core) -> Box<dyn CallerPage>;

    /// The starter lane of the caller's core, for the program that opened
    /// the core; none for any other caller, who may not open it.
    fn starter_lane(&self) -> Option<Arc<StarterLane>>;
}

/// How a caller left.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gone {
    /// It was signed out, or expired.
    Ended,
    /// What it is could not be read.
    Unread,
}

/// A caller's page while its synchronization channel lives: what the mount
/// adds to the core's parts of the product state for it, and what the page
/// says it shows.
pub trait CallerPage {
    /// The one part the page is sent while it may see nothing else, which
    /// the channel sends as it opens; none while it sees every part it
    /// follows.
    fn only(&self) -> Option<Part>;

    /// `part`, one of the mount's own, as the message that carries it now;
    /// none when the page is sent nothing of it.
    fn read<'a>(&'a self, core: &'a Core, part: &'a Part) -> LocalBoxFuture<'a, Result<Option<SyncEvent>, StateError>>;

    /// Learns `event` as the page is sent it, and answers the parts to send
    /// after it, which the page may see from then on.
    fn sent(&mut self, event: &SyncEvent) -> Vec<Part>;

    /// The page shows `conversation`, focused and visible, or none, and is
    /// in front of the user or not.
    fn show(&self, conversation: Option<ConversationId>, front: bool);

    /// The user acted on the page.
    fn used(&self);
}

/// The caller of a request.
#[derive(Clone)]
pub struct CallerOf(pub Arc<dyn Caller>);

impl<S: Send + Sync> FromRequestParts<S> for CallerOf {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, ApiError> {
        parts
            .extensions
            .get::<CallerOf>()
            .cloned()
            .ok_or_else(ApiError::unauthenticated)
    }
}

/// The principal of the core a request reaches, its caller's.
#[derive(Debug, Clone)]
pub struct Principal(pub UserId);

impl<S: Send + Sync> FromRequestParts<S> for Principal {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, ApiError> {
        let CallerOf(caller) = CallerOf::from_request_parts(parts, state).await?;
        Ok(Self(caller.principal().clone()))
    }
}

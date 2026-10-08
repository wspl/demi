//! The times and bounds the shard and the edge run with, which the
//! configuration sets and tests shorten (`backend.md` § Configuration).

use std::time::Duration;

/// When Demi reclaims what a conversation uses on a Host
/// (`resource-lifecycle.md` § Idle window). Tests shorten the times.
#[derive(Debug, Clone, Copy)]
pub struct LifecycleTuning {
    /// How long a conversation, or every conversation using the Cloud, stays
    /// inactive before its Host's resources are reclaimed.
    pub idle_window: Duration,
    /// How often a conversation's idle watch reads its activity.
    pub idle_poll: Duration,
}

impl Default for LifecycleTuning {
    fn default() -> Self {
        Self {
            idle_window: Duration::from_secs(60 * 60),
            idle_poll: Duration::from_secs(30),
        }
    }
}

/// How the backend serves conversations (`runtime.md` § Order and delivery,
/// `usage-and-quota.md` § Rate limit). Tests lower the bounds.
#[derive(Debug, Clone, Copy)]
pub struct ConversationTuning {
    /// The most frames a conversation socket's outbox holds; a client that
    /// falls this far behind is disconnected as lagging.
    pub outbox_frames: usize,
    /// The provider requests a user's conversations may start in any minute.
    pub requests_per_minute: usize,
    /// Whether the backend asks the conversation's model for a title
    /// (`product.md` § Conversation titles); off, a title stays the one the
    /// first message gives. A test whose scripted vendor answers only the
    /// turns turns it off.
    pub titles: bool,
    /// The least time between two indexings of one conversation for search
    /// (`storage.md` § Search index); a change within it is indexed once it
    /// has passed.
    pub search_interval: Duration,
}

impl Default for ConversationTuning {
    fn default() -> Self {
        Self {
            outbox_frames: demi_agent_server::ServerConfig::default().outbox_frames,
            requests_per_minute: demi_backend_providers::usage::rate_limit::REQUESTS_PER_WINDOW,
            titles: true,
            search_interval: Duration::from_secs(2),
        }
    }
}

/// How the backend times its sockets to a page, the synchronization channel
/// and the conversation sockets (`backend.md` § Page synchronization).
/// Tests shorten the times.
#[derive(Debug, Clone, Copy)]
pub struct PageTuning {
    /// A socket that has sent nothing for this long sends a heartbeat, so
    /// that its page can tell it from a dead one.
    pub heartbeat: Duration,
    /// How long a socket's close frame waits for a page that does not read;
    /// a page that has not taken it by then loses the connection without it
    /// (`backend.md` § Startup and shutdown).
    pub close_wait: Duration,
}

impl Default for PageTuning {
    fn default() -> Self {
        Self {
            heartbeat: Duration::from_secs(30),
            close_wait: Duration::from_secs(1),
        }
    }
}

/// How the backend treats runner connections (`runner.md` § Connection and
/// identity, `backend.md` § Authentication and ownership). Tests shorten
/// the times.
#[derive(Debug, Clone, Copy)]
pub struct RunnerTuning {
    /// A connection that sends no hello within this is closed.
    pub hello_deadline: Duration,
    /// How long a pairing code lives before its waiting runner gets a new one.
    pub claim_lifetime: Duration,
    /// How many pairing codes one user may try within a minute.
    pub claims_per_minute: usize,
    /// How often a connected runner is asked whether it is there; none turns
    /// liveness off.
    pub ping: Option<Duration>,
    /// How long the connection that holds a device has to answer a ping
    /// when a new hello for the device arrives, before it gives way.
    pub probe: Duration,
    /// How long a device shows as updating after the 409 that sent its
    /// runner to an update, unless a hello comes first.
    pub updating: Duration,
}

impl Default for RunnerTuning {
    fn default() -> Self {
        Self {
            hello_deadline: Duration::from_secs(30),
            claim_lifetime: Duration::from_secs(10 * 60),
            claims_per_minute: 10,
            ping: Some(demi_backend_remote_host::PING_INTERVAL),
            probe: Duration::from_secs(5),
            updating: Duration::from_secs(5 * 60),
        }
    }
}

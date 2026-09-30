//! How the backend runs each user's Cloud (`managed-hosts.md` § Lifecycle
//! and capacity).

use std::time::Duration;

/// The Cloud's times and limits. Tests shorten the times and lower the
/// capacity.
#[derive(Debug, Clone, Copy)]
pub struct CloudTuning {
    /// How often a running Cloud's maintenance and idle watch look at it.
    pub sweep: Duration,
    /// How long a running Cloud goes between checkpoints.
    pub checkpoint_interval: Duration,
    /// How long a Cloud runs before it is stopped, unless a turn is in
    /// flight.
    pub lifetime_cap: Duration,
    /// How long a Cloud's runner has to connect after a boot, or to
    /// reconnect during a recovery.
    pub runner_connection: Duration,
    /// How many runtime losses within `crash_loop_window` stop the
    /// Cloud's automatic boots.
    pub crash_loop_deaths: u32,
    pub crash_loop_window: Duration,
    /// How long a Cloud's runner has to flush its filesystems before a save.
    pub sync_timeout: Duration,
    /// How long a reset, or the lifetime cap, waits for the work it stops to
    /// let go.
    pub reset_hold: Duration,
    /// The largest system and home filesystems a Cloud may grow to, in
    /// bytes.
    pub system_quota: u64,
    pub home_quota: u64,
    /// How many Clouds, across every user, may be booting, running, saving
    /// or resetting at once.
    pub capacity: usize,
}

impl Default for CloudTuning {
    fn default() -> Self {
        const GIB: u64 = 1 << 30;
        Self {
            sweep: Duration::from_secs(30),
            checkpoint_interval: Duration::from_secs(15 * 60),
            lifetime_cap: Duration::from_secs(24 * 60 * 60),
            runner_connection: Duration::from_secs(60),
            crash_loop_deaths: 3,
            crash_loop_window: Duration::from_secs(10 * 60),
            sync_timeout: Duration::from_secs(5),
            reset_hold: Duration::from_secs(30),
            system_quota: 16 * GIB,
            home_quota: 32 * GIB,
            capacity: 16,
        }
    }
}

//! The backend's resource sample: one line a minute of what it holds, so that
//! a memory that grows shows as a series of numbers, and so does the count of
//! what holds it.
//!
//! The rules of the log (`docs/decisions/0060-one-log-tells-what-happened.md`)
//! cover units of work, waits, state changes and choices of path; a sample is
//! none of them, and it is logged for the one problem the rules cannot say
//! anything of, a backend that grows until the system ends it.

use std::time::Duration;

use demi_backend_database::server_log::ServerLog;
use demi_backend_user_shard::shard::Shards;
use demi_backend_user_shard::shard::lane::{open_connections, open_lanes};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

/// How often the backend samples itself. A line a minute is 1,440 lines a
/// day, a small share of the log's, and a rise in memory shows within
/// minutes of it.
const EVERY: Duration = Duration::from_secs(60);

/// Logs a sample every minute until the task ends with the backend.
pub(crate) async fn sample(shards: Shards, log: ServerLog) {
    let mut system = System::new();
    let mut ticks = tokio::time::interval_at(tokio::time::Instant::now() + EVERY, EVERY);
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        ticks.tick().await;
        let census = shards.census().await;
        tracing::info!(
            event = "backend.sampled",
            rss_mb = resident_mb(&mut system),
            shards = census.shards,
            live_trees = census.live_trees,
            live_sessions = census.live_sessions,
            running_turns = census.running_turns,
            page_connections = open_connections(),
            lanes = open_lanes(),
            log_queue = log.queued(),
            runners_online = census.runners_online,
            "the backend's resources were sampled"
        );
    }
}

/// This process's resident memory in whole mebibytes; none when the system
/// does not say.
fn resident_mb(system: &mut System) -> Option<u64> {
    let pid = sysinfo::get_current_pid().ok()?;
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        true,
        ProcessRefreshKind::nothing().with_memory(),
    );
    Some(system.process(pid)?.memory() / (1024 * 1024))
}

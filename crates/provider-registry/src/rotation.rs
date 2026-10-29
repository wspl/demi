//! Account rotation: which of an entry's accounts an agent takes under In Order
//! and Smart.
//!
//! The module tracks:
//!
//! - The accounts used up until their moment.
//! - How many agents run a turn on each account, so that agents started
//!   together spread over the accounts before their snapshots change. An idle
//!   agent keeps its account without counting on it.
//!
//! An agent keeps its account once it took one, because the vendor's prompt
//! cache belongs to the account: a 300K conversation that moves to another
//! account writes its 300K again there. It takes another only in these cases:
//!
//! - Its account fails with `usage_limit`.
//! - A compaction ran, whose next request writes its prefix anew in any case.
//! - Its account is removed, or the rotation changes.
//!
//! Smart spends first the quota a reset would take soonest, since what is
//! left at a reset is paid for and lost. Each account's longest window, its
//! week on a Claude or Codex plan, has an urgency: the share left divided by
//! the share of its period left. The most urgent account wins, unless one of
//! its shorter windows is nearly full while another account's is not.
//!
//! For example, Smart compares an account at 70% of its week with one day to
//! go (urgency 0.3 / (1/7) = 2.1) and one at 10% with six days to go
//! (0.9 / (6/7) = 1.05): the agent takes the first, whose 30% would be gone
//! tomorrow. With its 5-hour window 85% used, the agent takes the second.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use demi_shared_types::{Model, QuotaSnapshot, QuotaWindow, QuotaWindowName, Rotation, Timestamp};
use demi_web_api_protocol::ids::{CredentialId, ProviderId};

/// How long an account whose failure and quota name no moment stays used up.
pub const USED_UP_FOR_MS: i64 = 30 * 60 * 1000;

/// A short window used this much leaves its account out while another
/// account's short windows are below it: an agent that takes the account
/// would soon meet its limit in the middle of a turn and move, writing its
/// prompt cache anew on the next account.
const SHORT_WINDOW_FULL: f64 = 0.8;

/// Accounts whose urgency is within this share of the best are equal, and
/// the one fewer agents run a turn on is taken, so that agents started
/// together spread before their snapshots change.
const EQUAL_URGENCY: f64 = 0.3;

type Account = (ProviderId, CredentialId);

/// One account of an entry as a choice reads it, in the list's order.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub id: CredentialId,
    pub label: String,
    pub snapshot: Option<QuotaSnapshot>,
}

/// What a choice found.
#[derive(Debug)]
pub enum Choice {
    /// The account the agent takes, counted among those running on it
    /// until the count drops.
    Account(Running),
    /// Every account is used up; the earliest moment one can be used again,
    /// when known.
    AllUsedUp { earliest: Option<Timestamp> },
}

/// The rotation state of every entry this process serves.
#[derive(Debug, Default)]
pub struct AccountRotation {
    /// A `std` mutex: no section awaits.
    state: Mutex<State>,
}

#[derive(Debug, Default)]
struct State {
    /// The accounts used up, each until the moment it can be used again.
    used_up: HashMap<Account, Timestamp>,
    /// How many agents run a turn on each account. An idle agent keeps
    /// its account, but counts on none.
    running: HashMap<Account, usize>,
}

/// An agent counted among those running a turn on an account, until it
/// drops: Smart spreads agents of equal urgency by these counts.
#[derive(Debug)]
pub struct Running {
    rotation: Arc<AccountRotation>,
    account: Account,
}

impl Running {
    pub fn account(&self) -> &CredentialId {
        &self.account.1
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        let mut state = self.rotation.lock();
        if let Some(count) = state.running.get_mut(&self.account) {
            *count -= 1;
            if *count == 0 {
                state.running.remove(&self.account);
            }
        }
    }
}

impl AccountRotation {
    pub fn new() -> Arc<Self> {
        Arc::default()
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        // A section changes one count or one mark, so a poisoned map is
        // whole.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Takes an account of `entry` among `candidates` for an agent of
    /// `model` under `rotation`, and counts the agent as running on it.
    /// Manual takes none: its agents use the active account.
    pub fn take(
        self: &Arc<Self>,
        entry: &ProviderId,
        rotation: Rotation,
        candidates: &[Candidate],
        model: &Model,
        now: Timestamp,
    ) -> Option<Choice> {
        let mut state = self.lock();
        let rooms = rooms(&state, entry, candidates, model, now);
        let (mode, picked) = match rotation {
            Rotation::Manual => return None,
            Rotation::InOrder => ("order", rooms.iter().position(|room| room.usable()).map(|index| (index, 1))),
            Rotation::Smart => {
                let running = |index: usize| {
                    let key = (entry.clone(), candidates[index].id.clone());
                    state.running.get(&key).copied().unwrap_or(0)
                };
                ("smart", most_urgent(&rooms, running))
            }
        };
        let usable = rooms.iter().filter(|room| room.usable()).count();
        let Some((index, tied)) = picked else {
            let earliest = earliest(&rooms);
            tracing::info!(
                event = "account.chosen",
                provider = %entry,
                mode,
                candidates = candidates.len(),
                usable,
                reason = "all_used_up",
                earliest_ms = earliest.map(|moment| moment.as_millisecond()),
                "no account of the entry can be used now"
            );
            return Some(Choice::AllUsedUp { earliest });
        };
        let account = (entry.clone(), candidates[index].id.clone());
        let running = state.running.entry(account.clone()).or_default();
        // The conversation the choice is for is in the span of its caller.
        tracing::info!(
            event = "account.chosen",
            provider = %entry,
            account = %account.1,
            mode,
            candidates = candidates.len(),
            usable,
            urgency_pct = match rooms[index] {
                Room::Left { urgency, .. } => Some((urgency * 100.0).round().clamp(0.0, 10_000.0) as u64),
                Room::UsedUp(_) => None,
            },
            running = *running as u64,
            reason = match (rotation, tied) {
                (Rotation::InOrder, _) => "first_usable",
                (_, 1) => "most_urgent",
                _ => "fewest_running_among_equal_urgency",
            },
            "an account was chosen for an agent"
        );
        *running += 1;
        Some(Choice::Account(Running {
            rotation: self.clone(),
            account,
        }))
    }

    /// Counts an agent that runs a turn on `account` of `entry`, the one
    /// it keeps, as running on it.
    pub fn run(self: &Arc<Self>, entry: &ProviderId, account: &CredentialId) -> Running {
        let account = (entry.clone(), account.clone());
        *self.lock().running.entry(account.clone()).or_default() += 1;
        Running {
            rotation: self.clone(),
            account,
        }
    }

    /// The account Smart takes among `candidates` for `model`; none when
    /// every one is used up. It counts nothing: *Continue with* offers it to
    /// a Manual entry.
    pub fn most_urgent(
        &self,
        entry: &ProviderId,
        candidates: &[Candidate],
        model: &Model,
        now: Timestamp,
    ) -> Option<CredentialId> {
        let state = self.lock();
        let rooms = rooms(&state, entry, candidates, model, now);
        let running = |index: usize| {
            let key = (entry.clone(), candidates[index].id.clone());
            state.running.get(&key).copied().unwrap_or(0)
        };
        most_urgent(&rooms, running).map(|(index, _)| candidates[index].id.clone())
    }

    /// When every account among `candidates` is used up for `model`, the
    /// earliest moment one can be used again, when known; none while one
    /// can be used.
    pub fn all_used_up(
        &self,
        entry: &ProviderId,
        candidates: &[Candidate],
        model: &Model,
        now: Timestamp,
    ) -> Option<Option<Timestamp>> {
        let rooms = rooms(&self.lock(), entry, candidates, model, now);
        (!rooms.iter().any(Room::usable)).then(|| earliest(&rooms))
    }

    /// Whether `account` of `entry` is marked used up at `now`.
    pub fn is_used_up(&self, entry: &ProviderId, account: &CredentialId, now: Timestamp) -> bool {
        self.lock()
            .used_up
            .get(&(entry.clone(), account.clone()))
            .is_some_and(|until| *until > now)
    }

    /// Marks `account` of `entry` used up until `until`.
    pub fn mark_used_up(&self, entry: &ProviderId, account: &CredentialId, until: Timestamp) {
        self.lock()
            .used_up
            .insert((entry.clone(), account.clone()), until);
    }

    /// Forgets the marks of a deleted entry's accounts; their running
    /// counts go with the agents that run on them.
    pub fn forget_entry(&self, entry: &ProviderId) {
        self.lock().used_up.retain(|(held, _), _| held != entry);
    }
}

/// An account's standing for one model.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Room {
    /// Used up until the moment, when known.
    UsedUp(Option<Timestamp>),
    /// The urgency of its longest window, and whether a shorter window is
    /// nearly full.
    Left { urgency: f64, short_full: bool },
}

impl Room {
    fn usable(&self) -> bool {
        matches!(self, Self::Left { .. })
    }
}

fn rooms(
    state: &State,
    entry: &ProviderId,
    candidates: &[Candidate],
    model: &Model,
    now: Timestamp,
) -> Vec<Room> {
    candidates
        .iter()
        .map(|candidate| {
            let marked = state
                .used_up
                .get(&(entry.clone(), candidate.id.clone()))
                .filter(|until| **until > now);
            match marked {
                Some(until) => Room::UsedUp(Some(*until)),
                None => account_room(candidate.snapshot.as_ref(), model, now),
            }
        })
        .collect()
}

/// The standing of an account whose snapshot is `snapshot`, for `model`,
/// now.
///
/// - A window whose share is unknown says nothing of the account's room, but
///   its period still decides which windows are short. One past its reset
///   is unused.
/// - A window used up before its reset uses the account up until the latest
///   such reset.
/// - The urgency is that of the windows with the longest period, the
///   smallest when several have it: the share left divided by the share of
///   its period left. A window not yet started, or without a reset time or a
///   period, has an urgency of 1, as has an account without a snapshot.
/// - A window with a shorter period is short; one used `SHORT_WINDOW_FULL`
///   or more is nearly full. A window without a period beside one with a
///   period only says whether the account is used up.
fn account_room(snapshot: Option<&QuotaSnapshot>, model: &Model, now: Timestamp) -> Room {
    let windows: Vec<&QuotaWindow> = snapshot
        .into_iter()
        .flat_map(|snapshot| &snapshot.windows)
        .filter(|window| applies(window, model))
        .collect();
    let longest = windows
        .iter()
        .filter_map(|window| window.period_seconds.filter(|period| *period > 0))
        .max();
    let mut urgency = f64::INFINITY;
    let mut short_full = false;
    let mut used_up: Option<Option<Timestamp>> = None;
    for window in windows {
        let Some(percent) = window.used_percent else {
            continue;
        };
        let reset = window.resets_at.filter(|reset| *reset > now);
        let used = if window.resets_at.is_some() && reset.is_none() {
            // Its period ended: what it showed was used is free again.
            0.0
        } else {
            (percent / 100.0).clamp(0.0, 1.0)
        };
        if used >= 1.0 {
            used_up = Some(match used_up {
                Some(Some(latest)) => reset.map(|reset| reset.max(latest)),
                Some(None) | None => reset,
            });
            continue;
        }
        let period = window.period_seconds.filter(|period| *period > 0);
        match (period, longest) {
            (Some(period), Some(longest)) if period < longest => {
                short_full |= used >= SHORT_WINDOW_FULL;
            }
            _ if period == longest => {
                let window_urgency = match (reset, period) {
                    (Some(reset), Some(period)) => {
                        let left_ms = (reset.as_millisecond() - now.as_millisecond()) as f64;
                        (1.0 - used) / (left_ms / (period as f64 * 1000.0)).min(1.0)
                    }
                    _ => 1.0,
                };
                urgency = urgency.min(window_urgency);
            }
            _ => {}
        }
    }
    match used_up {
        Some(until) => Room::UsedUp(until),
        None => Room::Left {
            urgency: if urgency.is_finite() { urgency } else { 1.0 },
            short_full,
        },
    }
}

/// Whether `window` applies to `model`: every window but one scoped to
/// another model, which the scope names by the model's family name, such
/// as `Sonnet`.
fn applies(window: &QuotaWindow, model: &Model) -> bool {
    let Some(scope) = &window.scope else {
        return true;
    };
    let Some(name) = scope.label.as_deref().filter(|_| scope.kind == "model") else {
        return true;
    };
    let name = name.to_lowercase();
    model.name.to_lowercase().contains(&name) || model.id.to_lowercase().contains(&name)
}

/// The index of the account Smart takes, and how many accounts were equal
/// in urgency to it.
///
/// - An account with a nearly full short window is left out while another
///   account has none.
/// - Among the accounts within `EQUAL_URGENCY` of the best urgency, the one
///   `running` says fewest agents run a turn on wins.
/// - Then the first in the list wins.
fn most_urgent(rooms: &[Room], running: impl Fn(usize) -> usize) -> Option<(usize, usize)> {
    let left: Vec<(usize, f64, bool)> = rooms
        .iter()
        .enumerate()
        .filter_map(|(index, room)| match room {
            Room::Left { urgency, short_full } => Some((index, *urgency, *short_full)),
            Room::UsedUp(_) => None,
        })
        .collect();
    let open = left.iter().any(|(_, _, short_full)| !short_full);
    let eligible: Vec<&(usize, f64, bool)> = left
        .iter()
        .filter(|(_, _, short_full)| !open || !short_full)
        .collect();
    let best = eligible.iter().map(|(_, urgency, _)| *urgency).fold(f64::NAN, f64::max);
    let equal: Vec<&(usize, f64, bool)> = eligible
        .into_iter()
        .filter(|(_, urgency, _)| *urgency >= best * (1.0 - EQUAL_URGENCY))
        .collect();
    let tied = equal.len();
    equal
        .into_iter()
        .min_by_key(|(index, _, _)| (running(*index), *index))
        .map(|(index, _, _)| (*index, tied))
}

/// The earliest moment an account of `rooms`, all used up, can be used
/// again; none when no account names one.
fn earliest(rooms: &[Room]) -> Option<Timestamp> {
    rooms
        .iter()
        .filter_map(|room| match room {
            Room::UsedUp(until) => *until,
            Room::Left { .. } => None,
        })
        .min()
}

/// What names the window `snapshot` shows used up for a request of
/// `model`, the one that resets at `moment` when several are; a window on
/// another model is not the one the request reached. The line that says an
/// agent moved names it.
pub fn window_used_up(snapshot: &QuotaSnapshot, model: &Model, moment: Option<Timestamp>) -> Option<QuotaWindowName> {
    let used_up: Vec<&QuotaWindow> = snapshot
        .windows
        .iter()
        .filter(|window| window.used_percent.is_some_and(|percent| percent >= 100.0) && applies(window, model))
        .collect();
    used_up
        .iter()
        .find(|window| moment.is_some() && window.resets_at == moment)
        .or(used_up.first())
        .map(|window| window.name())
}

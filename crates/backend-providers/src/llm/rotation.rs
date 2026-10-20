//! Account rotation: which of an entry's accounts an agent takes under In Order
//! and Balanced.
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
//! For example, Balanced compares a Max account whose 5-hour window is 80% used
//! with 3 hours to go (room 0.2 / 0.6 = 0.33) and a Pro account at 10% of the
//! same window with 4 hours to go (room 0.9 / 0.8 = 1.1): the agent takes the
//! Pro account.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use demi_shared_types::{Model, QuotaSnapshot, QuotaWindow, QuotaWindowName, Rotation, Timestamp};
use demi_web_api_protocol::ids::{CredentialId, ProviderId};

/// How long an account whose failure and quota name no moment stays used up.
pub const USED_UP_FOR_MS: i64 = 30 * 60 * 1000;

/// A window used beyond this share leaves its account out while another
/// account has room.
const NEARLY_USED_UP: f64 = 0.9;

/// Accounts whose room is within this share of the best are equal, and the
/// one fewer agents run a turn on is taken.
const EQUAL_ROOM: f64 = 0.1;

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
/// drops: Balanced spreads agents of equal room by these counts.
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
        let index = match rotation {
            Rotation::Manual => return None,
            Rotation::InOrder => rooms.iter().position(|room| room.usable()),
            Rotation::Balanced => {
                let running = |index: usize| {
                    let key = (entry.clone(), candidates[index].id.clone());
                    state.running.get(&key).copied().unwrap_or(0)
                };
                most_room(&rooms, running)
            }
        };
        let Some(index) = index else {
            return Some(Choice::AllUsedUp {
                earliest: earliest(&rooms),
            });
        };
        let account = (entry.clone(), candidates[index].id.clone());
        *state.running.entry(account.clone()).or_default() += 1;
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

    /// The account with the most room among `candidates` for `model`, by
    /// Balanced's rule; none when every one is used up. It counts nothing:
    /// *Continue with* offers it to a Manual entry.
    pub fn most_room(
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
        most_room(&rooms, running).map(|index| candidates[index].id.clone())
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

/// An account's room for one model.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Room {
    /// Used up until the moment, when known.
    UsedUp(Option<Timestamp>),
    /// Its tightest window's room, and the largest share any of its windows
    /// has used.
    Left { room: f64, most_used: f64 },
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

/// The room of an account whose snapshot is `snapshot`, for `model`, now: its
/// smallest window's, each window's being the share left divided by the share
/// of its period left.
///
/// - A window not yet started, past its reset, or without a reset time or a
///   period has a room of 1.
/// - A window whose share is unknown says nothing.
/// - A window used up before its reset uses the account up until the latest
///   such reset.
fn account_room(snapshot: Option<&QuotaSnapshot>, model: &Model, now: Timestamp) -> Room {
    let windows = snapshot
        .into_iter()
        .flat_map(|snapshot| &snapshot.windows)
        .filter(|window| applies(window, model));
    let mut room = f64::INFINITY;
    let mut most_used: f64 = 0.0;
    let mut used_up: Option<Option<Timestamp>> = None;
    for window in windows {
        let Some(percent) = window.used_percent else {
            continue;
        };
        let used = (percent / 100.0).clamp(0.0, 1.0);
        let reset = window.resets_at.filter(|reset| *reset > now);
        if window.resets_at.is_some() && reset.is_none() {
            // Its period ended: what it showed was used is free again.
            room = room.min(1.0);
            continue;
        }
        if used >= 1.0 {
            used_up = Some(match used_up {
                Some(Some(latest)) => reset.map(|reset| reset.max(latest)),
                Some(None) | None => reset,
            });
            continue;
        }
        most_used = most_used.max(used);
        let window_room = match (reset, window.period_seconds) {
            (Some(reset), Some(period)) if period > 0 => {
                let left_ms = (reset.as_millisecond() - now.as_millisecond()) as f64;
                let period_left = (left_ms / (period as f64 * 1000.0)).min(1.0);
                (1.0 - used) / period_left
            }
            _ => 1.0,
        };
        room = room.min(window_room);
    }
    match used_up {
        Some(until) => Room::UsedUp(until),
        None => Room::Left {
            room: if room.is_finite() { room } else { 1.0 },
            most_used,
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

/// The index of the account with the most room.
///
/// - A window over 90% used leaves its account out while another account has
///   room.
/// - Among the accounts within a tenth of the best room, the one `running` says
///   fewest agents run a turn on wins.
/// - Then the first in the list wins.
fn most_room(rooms: &[Room], running: impl Fn(usize) -> usize) -> Option<usize> {
    let left: Vec<(usize, f64, f64)> = rooms
        .iter()
        .enumerate()
        .filter_map(|(index, room)| match room {
            Room::Left { room, most_used } => Some((index, *room, *most_used)),
            Room::UsedUp(_) => None,
        })
        .collect();
    let roomy = left.iter().any(|(_, _, most_used)| *most_used <= NEARLY_USED_UP);
    let eligible: Vec<&(usize, f64, f64)> = left
        .iter()
        .filter(|(_, _, most_used)| !roomy || *most_used <= NEARLY_USED_UP)
        .collect();
    let best = eligible.iter().map(|(_, room, _)| *room).fold(f64::NAN, f64::max);
    eligible
        .into_iter()
        .filter(|(_, room, _)| *room >= best * (1.0 - EQUAL_ROOM))
        .min_by_key(|(index, _, _)| (running(*index), *index))
        .map(|(index, _, _)| *index)
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

#[cfg(test)]
mod tests {
    use demi_shared_types::{QuotaKind, QuotaScope};

    use super::*;

    const HOUR: i64 = 3_600_000;

    fn now() -> Timestamp {
        "2026-09-24T08:00:00.000Z".parse().unwrap()
    }

    fn at(hours: f64) -> Timestamp {
        Timestamp::from_millisecond(now().as_millisecond() + (hours * HOUR as f64) as i64).unwrap()
    }

    fn window(used: f64, resets_in_hours: Option<f64>, period_hours: Option<u64>) -> QuotaWindow {
        QuotaWindow {
            id: "w".into(),
            kind: QuotaKind::Limit,
            used_percent: Some(used),
            used: None,
            limit: None,
            unit: None,
            resets_at: resets_in_hours.map(at),
            period_seconds: period_hours.map(|hours| hours * 3600),
            requested_at: None,
            severity: None,
            scope: None,
        }
    }

    fn account(id: &str, windows: Vec<QuotaWindow>) -> Candidate {
        Candidate {
            id: CredentialId::try_from(id).unwrap(),
            label: format!("{id}@example.com"),
            snapshot: Some(QuotaSnapshot {
                observed_at: now(),
                source: demi_shared_types::SnapshotSource::Probe,
                plan: None,
                account_label: None,
                windows,
            }),
        }
    }

    fn model(name: &str) -> Model {
        serde_json::from_value(serde_json::json!({
            "id": name.to_lowercase(), "name": name, "contextWindow": 200000,
            "outputLimit": null, "thinking": [], "acceptedExtensions": null
        }))
        .unwrap()
    }

    fn entry() -> ProviderId {
        ProviderId::try_from("entry").unwrap()
    }

    /// An Opus request on an account whose Sonnet and Opus windows are both
    /// used up reached the Opus one, which the account switch's line names,
    /// whichever the snapshot lists first.
    #[test]
    fn the_window_named_used_up_is_the_one_on_the_requests_model() {
        let on = |model: &str| {
            let mut window = window(100.0, None, Some(168));
            window.scope = Some(QuotaScope {
                kind: "model".into(),
                label: Some(model.into()),
            });
            window
        };
        let candidate = account("a", vec![window(40.0, None, Some(5)), on("Sonnet"), on("Opus")]);
        let snapshot = candidate.snapshot.unwrap();
        let named = window_used_up(&snapshot, &model("Claude Opus 4.8"), None).unwrap();
        assert_eq!(named.scope.and_then(|scope| scope.label).as_deref(), Some("Opus"));
        assert_eq!(window_used_up(&snapshot, &model("Claude Haiku 4.5"), None), None);
    }

    /// The account `rotation` takes, its agent kept running.
    fn take(
        rotation: &Arc<AccountRotation>,
        how: Rotation,
        accounts: &[Candidate],
        running: &mut Vec<Running>,
    ) -> String {
        match rotation.take(&entry(), how, accounts, &model("Opus"), now()) {
            Some(Choice::Account(taken)) => {
                let id = taken.account().to_string();
                running.push(taken);
                id
            }
            other => panic!("{other:?}"),
        }
    }

    /// The doc's example: a 5-hour window 80% used with 3 hours to go has
    /// 0.2 / 0.6 = 0.33, a weekly window 30% used with 2 days to go 2.4, so
    /// the account's room is 0.33 and one whose 5-hour window is 10% used
    /// with 4 hours to go (1.1) has more. A window not started has 1, and
    /// so has one without a reset; a used-up window leaves its account out,
    /// and a window over 90% used does while another account has room.
    /// Planted defect this catches: the largest window deciding, a soon
    /// reset counted as tight, or the 90% rule ignored. Microseconds.
    #[test]
    fn balanced_takes_the_account_whose_tightest_window_has_the_most_room() {
        let weekly = |used, days: f64| window(used, Some(days * 24.0), Some(7 * 24));
        let cases: Vec<(&str, Vec<Candidate>, &str)> = vec![
            (
                "the tightest window decides",
                vec![
                    account("max", vec![window(80.0, Some(3.0), Some(5)), weekly(30.0, 2.0)]),
                    account("pro", vec![window(10.0, Some(4.0), Some(5))]),
                ],
                "pro",
            ),
            (
                "a weekly window spent early is tighter than a fresh 5-hour one",
                vec![
                    account("max", vec![weekly(70.0, 6.0)]),
                    account("pro", vec![window(0.0, None, Some(5))]),
                ],
                "pro",
            ),
            (
                "a window that resets soon counts as roomy",
                vec![
                    account("max", vec![window(50.0, Some(4.5), Some(5))]),
                    account("pro", vec![window(85.0, Some(0.25), Some(5))]),
                ],
                "pro",
            ),
            (
                "a used-up window leaves the account out",
                vec![
                    account("max", vec![window(100.0, Some(4.0), Some(5))]),
                    account("pro", vec![weekly(85.0, 1.0)]),
                ],
                "pro",
            ),
            (
                "over 90% used is out while another has room",
                vec![
                    account("max", vec![window(95.0, Some(0.1), Some(5))]),
                    account("pro", vec![window(60.0, Some(4.0), Some(5))]),
                ],
                "pro",
            ),
            (
                "over 90% used counts when every account is",
                vec![
                    account("max", vec![window(99.0, Some(4.0), Some(5))]),
                    account("pro", vec![window(95.0, Some(4.0), Some(5))]),
                ],
                "pro",
            ),
            (
                "a window on another model does not count",
                vec![
                    account(
                        "max",
                        vec![QuotaWindow {
                            scope: Some(QuotaScope {
                                kind: "model".into(),
                                label: Some("Sonnet".into()),
                            }),
                            ..window(100.0, Some(48.0), Some(7 * 24))
                        }],
                    ),
                    account("pro", vec![window(40.0, Some(4.0), Some(5))]),
                ],
                "max",
            ),
        ];
        for (case, accounts, expected) in cases {
            let rotation = AccountRotation::new();
            let taken = take(&rotation, Rotation::Balanced, &accounts, &mut Vec::new());
            assert_eq!(taken, expected, "{case}");
        }
    }

    /// Agents started together spread over accounts of equal room, the
    /// first in the list first, and an agent whose turn ends no longer
    /// counts. Planted defect this catches: a choice that ignores the
    /// running agents, or a count never released. Microseconds.
    #[test]
    fn agents_started_together_spread_over_accounts_within_a_tenth_of_the_best() {
        let rotation = AccountRotation::new();
        let accounts = vec![
            account("max", vec![window(50.0, Some(5.0), Some(5))]),
            account("pro", vec![window(54.0, Some(5.0), Some(5))]),
            account("team", vec![window(80.0, Some(5.0), Some(5))]),
        ];
        let mut running = Vec::new();
        let taken: Vec<_> = (0..4)
            .map(|_| take(&rotation, Rotation::Balanced, &accounts, &mut running))
            .collect();
        assert_eq!(taken, ["max", "pro", "max", "pro"]);
        running.retain(|agent| agent.account().as_str() != "pro");
        assert_eq!(take(&rotation, Rotation::Balanced, &accounts, &mut running), "pro");
    }

    /// In Order takes the first account not used up, by its snapshot or by
    /// a failure's mark until its moment; with every one used up the
    /// choice names the earliest moment, and Manual takes none. Planted
    /// defect this catches: a mark that outlives its moment, or the latest
    /// moment named. Microseconds.
    #[test]
    fn in_order_takes_the_first_account_not_used_up_and_names_the_earliest_moment() {
        let rotation = AccountRotation::new();
        let accounts = vec![
            account("a", vec![window(100.0, Some(2.0), Some(5))]),
            account("b", vec![window(99.0, Some(4.0), Some(5))]),
            account("c", Vec::new()),
        ];
        let mut running = Vec::new();
        assert_eq!(take(&rotation, Rotation::InOrder, &accounts, &mut running), "b");
        rotation.mark_used_up(&entry(), &accounts[1].id, at(3.0));
        assert_eq!(take(&rotation, Rotation::InOrder, &accounts, &mut running), "c");
        rotation.mark_used_up(&entry(), &accounts[2].id, at(1.0));
        match rotation.take(&entry(), Rotation::InOrder, &accounts, &model("Opus"), now()) {
            Some(Choice::AllUsedUp { earliest }) => assert_eq!(earliest, Some(at(1.0))),
            other => panic!("{other:?}"),
        }
        // A mark's moment passed: the account is back.
        let later = at(1.5);
        match rotation.take(&entry(), Rotation::InOrder, &accounts, &model("Opus"), later) {
            Some(Choice::Account(taken)) => assert_eq!(taken.account().as_str(), "c"),
            other => panic!("{other:?}"),
        }
        assert!(rotation.take(&entry(), Rotation::Manual, &accounts, &model("Opus"), now()).is_none());
    }
}

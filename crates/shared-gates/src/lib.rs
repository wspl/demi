//! Named gates that serialize work across awaits.
//! An [`ActivityGate`] admits leases and quiets for reservations, a
//! [`SerialGate`] admits one holder at a time, and a [`KeyedSerialGate`]
//! keeps a serial gate for each key while anyone uses it; a [`RateWindow`]
//! admits so many attempts of each key within a sliding window. Every lease,
//! reservation and permit releases when dropped, and waiting gives up its
//! place when its future is dropped.
//!
//! A lease of an `ActivityGate` is one permit: work in progress, such as one
//! host access. A reservation takes every permit: a transition that needs the
//! gate quiet, such as a target switch. Entrants are served first in, first
//! out, so a waiting reservation holds back every later entrant, and trying to
//! reserve succeeds only when nothing holds the gate. The gate publishes its
//! state as a snapshot, including when demand last ended, which idle watches
//! read. A `SerialGate`'s permit can move into a task.
//!
//! The crate knows no user, conversation, device or other domain.

mod activity;
mod rate;
mod serial;

pub use activity::{ActivityGate, ActivityHub, GateLease, GateState, Purpose, Reservation};
pub use rate::RateWindow;
pub use serial::{KeyedPermit, KeyedSerialGate, SerialGate, SerialPermit};

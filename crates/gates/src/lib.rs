//! Named gates that serialize work across awaits (`concurrency.md` § Locks).
//! An [`ActivityGate`] admits leases and quiets for reservations, a
//! [`SerialGate`] admits one holder at a time, and a [`KeyedSerialGate`]
//! keeps a serial gate for each key while anyone uses it. Every lease,
//! reservation and permit releases when dropped, and waiting gives up its
//! place when its future is dropped.

mod activity;
mod serial;

pub use activity::{ActivityGate, ActivityHub, GateLease, GateState, Purpose, Reservation};
pub use serial::{KeyedPermit, KeyedSerialGate, SerialGate, SerialPermit};

//! A first-in, first-out semaphore with a published state: a lease is one
//! permit, a reservation is every permit. A reservation that is waiting has
//! taken the permits already free, so every later entrant waits behind it and
//! [`ActivityGate::try_enter`] is refused; giving up the wait returns them.

use std::sync::Arc;

use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore, watch},
    time::Instant,
};

/// Every permit a gate has: a reservation takes them all. It bounds the
/// leases held at once far above any real count, on every target.
const PERMITS: u32 = 1 << 24;

/// Why a lease holds a gate: what the user asked for, or work the system
/// does on its own, such as a checkpoint. Only demand keeps a resource from
/// being idle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    Demand,
    Maintenance,
}

/// What holds a gate, as its observers see it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GateState {
    /// Leases held for demand.
    pub demand: u32,
    /// Leases held for maintenance.
    pub maintenance: u32,
    /// Whether a reservation holds the gate.
    pub reserved: bool,
    /// When the last demand lease ended; none until one has.
    pub last_demand_end: Option<Instant>,
}

/// A counter the gates of one user bump on every change, so a watch that
/// waits for any of them to change wakes. It carries no state of its own: a
/// watch reads [`GateState::last_demand_end`], which a missed bump cannot
/// lose.
#[derive(Debug, Clone)]
pub struct ActivityHub(Arc<watch::Sender<u64>>);

impl ActivityHub {
    pub fn new() -> Self {
        Self(Arc::new(watch::Sender::new(0)))
    }

    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.0.subscribe()
    }

    fn bump(&self) {
        self.0.send_modify(|count| *count = count.wrapping_add(1));
    }
}

impl Default for ActivityHub {
    fn default() -> Self {
        Self::new()
    }
}

struct Inner {
    permits: Arc<Semaphore>,
    state: watch::Sender<GateState>,
    hub: Option<ActivityHub>,
}

impl Inner {
    fn publish(&self, change: impl FnOnce(&mut GateState)) {
        self.state.send_modify(change);
        if let Some(hub) = &self.hub {
            hub.bump();
        }
    }
}

/// Admission to one resource: many leases at once, or one reservation alone.
#[derive(Clone)]
pub struct ActivityGate(Arc<Inner>);

impl ActivityGate {
    pub fn new() -> Self {
        Self::gate(None)
    }

    /// A gate whose changes also bump `hub`.
    pub fn with_hub(hub: ActivityHub) -> Self {
        Self::gate(Some(hub))
    }

    fn gate(hub: Option<ActivityHub>) -> Self {
        Self(Arc::new(Inner {
            permits: Arc::new(Semaphore::new(PERMITS as usize)),
            state: watch::Sender::new(GateState::default()),
            hub,
        }))
    }

    /// Holds a lease once no reservation holds or waits for the gate ahead of
    /// this call.
    pub async fn enter(&self, purpose: Purpose) -> GateLease {
        let permit = self
            .0
            .permits
            .clone()
            .acquire_owned()
            .await
            .expect("a gate's semaphore is never closed");
        GateLease::admit(self.0.clone(), purpose, permit)
    }

    /// Holds a lease now, or none while a reservation holds or waits.
    pub fn try_enter(&self, purpose: Purpose) -> Option<GateLease> {
        let permit = self.0.permits.clone().try_acquire_owned().ok()?;
        Some(GateLease::admit(self.0.clone(), purpose, permit))
    }

    /// Holds the gate alone once every lease ahead of it has ended. Leases
    /// that arrive while it waits wait behind it.
    pub async fn reserve(&self) -> Reservation {
        let permits = self
            .0
            .permits
            .clone()
            .acquire_many_owned(PERMITS)
            .await
            .expect("a gate's semaphore is never closed");
        Reservation::hold(self.0.clone(), permits)
    }

    /// Holds the gate alone now, or none while anything holds or waits for it.
    pub fn try_reserve(&self) -> Option<Reservation> {
        let permits = self.0.permits.clone().try_acquire_many_owned(PERMITS).ok()?;
        Some(Reservation::hold(self.0.clone(), permits))
    }

    pub fn state(&self) -> GateState {
        *self.0.state.borrow()
    }

    pub fn subscribe(&self) -> watch::Receiver<GateState> {
        self.0.state.subscribe()
    }
}

impl Default for ActivityGate {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for ActivityGate {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_tuple("ActivityGate")
            .field(&self.state())
            .finish()
    }
}

/// One piece of work in progress; dropping it ends the work's hold.
pub struct GateLease {
    gate: Arc<Inner>,
    purpose: Purpose,
    /// Taken in `drop`, after the state no longer counts this lease, so a
    /// reservation it lets in never sees it.
    permit: Option<OwnedSemaphorePermit>,
}

impl GateLease {
    fn admit(gate: Arc<Inner>, purpose: Purpose, permit: OwnedSemaphorePermit) -> Self {
        gate.publish(|state| match purpose {
            Purpose::Demand => state.demand += 1,
            Purpose::Maintenance => state.maintenance += 1,
        });
        Self {
            gate,
            purpose,
            permit: Some(permit),
        }
    }

    pub fn purpose(&self) -> Purpose {
        self.purpose
    }
}

impl std::fmt::Debug for GateLease {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GateLease")
            .field("purpose", &self.purpose)
            .finish()
    }
}

impl Drop for GateLease {
    fn drop(&mut self) {
        let purpose = self.purpose;
        self.gate.publish(|state| match purpose {
            Purpose::Demand => {
                state.demand -= 1;
                if state.demand == 0 {
                    state.last_demand_end = Some(Instant::now());
                }
            }
            Purpose::Maintenance => state.maintenance -= 1,
        });
        drop(self.permit.take());
    }
}

/// The gate held alone, for a transition that needs it quiet; dropping it
/// admits the entrants waiting behind it.
pub struct Reservation {
    gate: Arc<Inner>,
    /// Taken in `drop`, after the state shows the gate free.
    permits: Option<OwnedSemaphorePermit>,
}

impl Reservation {
    fn hold(gate: Arc<Inner>, permits: OwnedSemaphorePermit) -> Self {
        gate.publish(|state| state.reserved = true);
        Self {
            gate,
            permits: Some(permits),
        }
    }
}

impl std::fmt::Debug for Reservation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Reservation").finish()
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        self.gate.publish(|state| state.reserved = false);
        drop(self.permits.take());
    }
}

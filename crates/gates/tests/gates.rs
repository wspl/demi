//! The gates' admission order, their published state, and their release on
//! drop, including of a waiting future.

use std::{sync::Arc, time::Duration};

use demi_gates::{ActivityGate, ActivityHub, KeyedSerialGate, Purpose, SerialGate};
use tokio::sync::Mutex;

/// Lets every ready task run, on the test's current-thread runtime.
async fn settle() {
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
}

#[tokio::test]
async fn a_reservation_drains_current_work_and_queues_new_work_until_it_ends() {
    let gate = ActivityGate::new();
    let first = gate.enter(Purpose::Demand).await;
    let second = gate.enter(Purpose::Demand).await;
    assert!(gate.try_reserve().is_none());
    let events = Arc::new(Mutex::new(Vec::new()));
    let writer = tokio::spawn({
        let gate = gate.clone();
        let events = events.clone();
        async move {
            let reservation = gate.reserve().await;
            events.lock().await.push("writer");
            reservation
        }
    });
    settle().await;
    // A waiting reservation holds back later entrants, and try_enter fails.
    assert!(gate.try_enter(Purpose::Demand).is_none());
    let entrant = tokio::spawn({
        let gate = gate.clone();
        let events = events.clone();
        async move {
            let lease = gate.enter(Purpose::Demand).await;
            events.lock().await.push("entrant");
            lease
        }
    });
    drop(first);
    settle().await;
    assert!(events.lock().await.is_empty());
    drop(second);
    let reservation = writer.await.unwrap();
    assert_eq!(*events.lock().await, ["writer"]);
    assert!(gate.state().reserved);
    drop(reservation);
    let lease = entrant.await.unwrap();
    assert_eq!(*events.lock().await, ["writer", "entrant"]);
    assert_eq!(gate.state().demand, 1);
    drop(lease);
    assert_eq!(gate.state().demand, 0);
}

#[tokio::test]
async fn giving_up_a_draining_reservation_admits_waiters_without_touching_leases() {
    let gate = ActivityGate::new();
    let held = gate.enter(Purpose::Demand).await;
    let writer = tokio::spawn({
        let gate = gate.clone();
        async move { gate.reserve().await }
    });
    settle().await;
    let entrant = tokio::spawn({
        let gate = gate.clone();
        async move { gate.enter(Purpose::Demand).await }
    });
    settle().await;
    assert!(!entrant.is_finished());
    writer.abort();
    assert!(writer.await.unwrap_err().is_cancelled());
    let lease = entrant.await.unwrap();
    assert_eq!(gate.state().demand, 2);
    drop(lease);
    drop(held);
    let reservation = gate.try_reserve().expect("an idle gate reserves");
    drop(reservation);
}

#[tokio::test]
async fn an_idle_reservation_excludes_entrants_and_a_given_up_wait_never_admits() {
    let gate = ActivityGate::new();
    let reservation = gate.try_reserve().expect("an idle gate reserves");
    assert!(gate.try_reserve().is_none());
    assert!(gate.try_enter(Purpose::Maintenance).is_none());
    let waiting = tokio::spawn({
        let gate = gate.clone();
        async move { gate.enter(Purpose::Demand).await }
    });
    settle().await;
    waiting.abort();
    assert!(waiting.await.unwrap_err().is_cancelled());
    drop(reservation);
    assert_eq!(gate.state().demand, 0);
    assert!(!gate.state().reserved);
    assert!(gate.try_reserve().is_some());
}

#[tokio::test(start_paused = true)]
async fn the_state_counts_leases_by_purpose_and_records_when_demand_ends() {
    let hub = ActivityHub::new();
    let mut bumps = hub.subscribe();
    let gate = ActivityGate::with_hub(hub.clone());
    let mut state = gate.subscribe();
    let maintenance = gate.try_enter(Purpose::Maintenance).unwrap();
    let demand = gate.enter(Purpose::Demand).await;
    assert!(state.has_changed().unwrap());
    let seen = *state.borrow_and_update();
    assert_eq!((seen.demand, seen.maintenance, seen.last_demand_end), (1, 1, None));
    tokio::time::advance(Duration::from_secs(5)).await;
    let ended = tokio::time::Instant::now();
    drop(demand);
    assert_eq!(gate.state().last_demand_end, Some(ended));
    // Maintenance ending is not demand ending.
    tokio::time::advance(Duration::from_secs(5)).await;
    drop(maintenance);
    assert_eq!(gate.state().last_demand_end, Some(ended));
    assert_eq!(gate.state().maintenance, 0);
    assert!(bumps.has_changed().unwrap());
    assert!(*bumps.borrow_and_update() >= 4);
}

#[tokio::test]
async fn a_serial_gate_admits_in_arrival_order_and_its_permit_moves_into_a_task() {
    let gate = SerialGate::new();
    let order = Arc::new(Mutex::new(Vec::new()));
    let first = gate.acquire().await;
    assert!(gate.try_acquire().is_none());
    let mut waiters = Vec::new();
    for index in 0..5 {
        let gate = gate.clone();
        let order = order.clone();
        waiters.push(tokio::spawn(async move {
            let permit = gate.acquire().await;
            order.lock().await.push(index);
            // The permit travels into the work it admits.
            tokio::spawn(async move {
                tokio::task::yield_now().await;
                drop(permit);
            })
            .await
            .unwrap();
        }));
        settle().await;
    }
    drop(first);
    for waiter in waiters {
        waiter.await.unwrap();
    }
    assert_eq!(*order.lock().await, [0, 1, 2, 3, 4]);
    assert!(gate.try_acquire().is_some());
}

#[tokio::test]
async fn keyed_gates_serialize_one_key_and_forget_it_once_nobody_uses_it() {
    let gates = KeyedSerialGate::<&str>::new();
    let a = gates.acquire("a").await;
    // Another key does not wait.
    let b = gates.acquire("b").await;
    assert_eq!(gates.len(), 2);
    let waiter = tokio::spawn({
        let gates = gates.clone();
        async move { gates.acquire("a").await }
    });
    settle().await;
    assert!(!waiter.is_finished());
    drop(b);
    assert_eq!(gates.len(), 1);
    drop(a);
    let again = waiter.await.unwrap();
    assert_eq!(gates.len(), 1);
    drop(again);
    assert!(gates.is_empty());
    // A waiter that gives up leaves nothing behind once the holder leaves.
    let held = gates.acquire("c").await;
    let quitter = tokio::spawn({
        let gates = gates.clone();
        async move { gates.acquire("c").await }
    });
    settle().await;
    quitter.abort();
    assert!(quitter.await.unwrap_err().is_cancelled());
    drop(held);
    assert!(gates.is_empty());
}

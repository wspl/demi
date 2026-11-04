//! A pair of runners' measurement of its peer against the relay, as a page
//! measures a device's (`packages/web/src/direct/measure.ts`), with the same
//! figures (`demi_runner_protocol::direct`): [`MEASURE_PROBES`] probes on
//! each path, [`PROBE_EVERY`] apart, each lost when not answered within
//! [`PROBE_LOST`].
//!
//! The direct path's probes go on the peer's probe channel, unordered and
//! never retransmitted, so a loss shows as one; the relay's go over TCP,
//! which loses none, and count only their round trips.

use std::collections::HashMap;
use std::time::Duration;

use demi_runner_protocol::direct::{MEASURE_PROBES, PROBE_EVERY, PROBE_LOST, WORSE_LATENCY, WORSE_LOST_PROBES};
use tokio::sync::mpsc;
use tokio::time::Instant;

/// One path a measurement probes: how a probe goes; its answers come on the
/// receiver given beside it.
pub trait ProbedPath: Send + Sync {
    fn send(&self, id: u32);
}

/// What one path's probes found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Figures {
    /// The median round trip of the probes answered; none when none was.
    pub latency: Option<Duration>,
    /// The probes not answered in time.
    pub lost: usize,
}

/// One path's probes of a measurement.
struct Run {
    sent: HashMap<u32, Instant>,
    rtts: Vec<Duration>,
}

impl Run {
    fn new() -> Self {
        Self { sent: HashMap::new(), rtts: Vec::new() }
    }

    fn probe(&mut self, path: &dyn ProbedPath, id: u32) {
        self.sent.insert(id, Instant::now());
        path.send(id);
    }

    fn answered(&mut self, id: u32) {
        // An answer after the limit came from a probe already counted lost.
        if let Some(at) = self.sent.remove(&id)
            && at.elapsed() <= PROBE_LOST
        {
            self.rtts.push(at.elapsed());
        }
    }

    fn done(&self) -> bool {
        self.rtts.len() == MEASURE_PROBES
    }

    fn figures(mut self) -> Figures {
        self.rtts.sort();
        let middle = self.rtts.len() / 2;
        let latency = match self.rtts.len() {
            0 => None,
            count if count % 2 == 1 => Some(self.rtts[middle]),
            _ => Some((self.rtts[middle - 1] + self.rtts[middle]) / 2),
        };
        Figures { latency, lost: MEASURE_PROBES - self.rtts.len() }
    }
}

/// Measures `direct` and `relay`, whose answers come on `direct_answers`
/// and `relay_answers`: both paths' figures, once every probe came back or
/// [`PROBE_LOST`] after the last was sent.
pub async fn measure(
    direct: &dyn ProbedPath,
    direct_answers: &mut mpsc::UnboundedReceiver<u32>,
    relay: &dyn ProbedPath,
    relay_answers: &mut mpsc::UnboundedReceiver<u32>,
) -> (Figures, Figures) {
    let mut direct_run = Run::new();
    let mut relay_run = Run::new();
    let start = Instant::now();
    let mut sent = 0usize;
    let mut next_probe = start;
    let last = start + PROBE_EVERY * (MEASURE_PROBES as u32 - 1) + PROBE_LOST;
    loop {
        if sent == MEASURE_PROBES && direct_run.done() && relay_run.done() {
            break;
        }
        tokio::select! {
            () = tokio::time::sleep_until(next_probe), if sent < MEASURE_PROBES => {
                let id = sent as u32;
                direct_run.probe(direct, id);
                relay_run.probe(relay, id);
                sent += 1;
                next_probe = start + PROBE_EVERY * sent as u32;
            }
            Some(id) = direct_answers.recv() => direct_run.answered(id),
            Some(id) = relay_answers.recv() => relay_run.answered(id),
            () = tokio::time::sleep_until(last) => break,
        }
    }
    (direct_run.figures(), relay_run.figures())
}

/// Whether the direct path is worse than the relay: it lost more than
/// [`WORSE_LOST_PROBES`] of its probes, or its latency is more than
/// [`WORSE_LATENCY`] above the relay's.
pub fn direct_worse(direct: Figures, relay: Figures) -> bool {
    if direct.lost > WORSE_LOST_PROBES {
        return true;
    }
    match (direct.latency, relay.latency) {
        (Some(direct), Some(relay)) => direct > relay + WORSE_LATENCY,
        // A relay that answered nothing is no figure to compare with.
        _ => false,
    }
}

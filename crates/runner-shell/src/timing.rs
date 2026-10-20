//! What `time` reports of a pipeline in a job: the processor time of the
//! processes the pipeline started, none of the runner's (`crates/runner-shell/src/process_builtins.rs`).

use std::sync::{Arc, Mutex, PoisonError};

use brush_core::execution_host::{ProcessorTime, Timing};

/// The line a report adds when the pipeline ran a builtin or a standard
/// utility, whose work the measure leaves out.
const UNCOUNTED: &str = "(work inside the runner not counted)";

/// What one timed pipeline has run so far.
#[derive(Default)]
struct Totals {
    time: ProcessorTime,
    in_runner: bool,
}

/// The totals of the pipelines being timed in one part of a job, which each
/// process and builtin started there counts towards. A std mutex: the job's
/// interpreter and utility threads share it for moments that never await.
#[derive(Clone, Default)]
pub(crate) struct Timings(Arc<Mutex<Vec<Arc<Mutex<Totals>>>>>);

impl Timings {
    /// The timings of a part of the job that starts now, such as a
    /// background task: the pipelines timed now, and none that its parent
    /// times from then on.
    pub(crate) fn part(&self) -> Self {
        Self(Arc::new(Mutex::new(self.current().0)))
    }

    /// What a process started now counts towards.
    pub(crate) fn current(&self) -> Counted {
        Counted(self.0.lock().unwrap_or_else(PoisonError::into_inner).clone())
    }

    /// Starts timing a pipeline, until the timing drops.
    pub(crate) fn start(&self) -> PipelineTiming {
        let totals = Arc::<Mutex<Totals>>::default();
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(totals.clone());
        PipelineTiming {
            totals,
            timings: self.clone(),
        }
    }

    /// Notes a builtin or standard utility, which runs inside the runner.
    pub(crate) fn builtin_started(&self) {
        for totals in self.0.lock().unwrap_or_else(PoisonError::into_inner).iter() {
            totals.lock().unwrap_or_else(PoisonError::into_inner).in_runner = true;
        }
    }
}

/// The timed pipelines a process counts towards: those timed when it started.
pub(crate) struct Counted(Vec<Arc<Mutex<Totals>>>);

impl Counted {
    /// Adds the processor time of the process, which has ended.
    pub(crate) fn add(&self, time: demi_runner_process::process::ProcessorTime) {
        for totals in &self.0 {
            let mut totals = totals.lock().unwrap_or_else(PoisonError::into_inner);
            totals.time.user += time.user;
            totals.time.system += time.system;
        }
    }
}

/// One timed pipeline's measure.
pub(crate) struct PipelineTiming {
    totals: Arc<Mutex<Totals>>,
    timings: Timings,
}

impl Timing for PipelineTiming {
    fn processor_time(&self) -> ProcessorTime {
        self.totals.lock().unwrap_or_else(PoisonError::into_inner).time
    }

    fn uncounted(&self) -> Option<&'static str> {
        let totals = self.totals.lock().unwrap_or_else(PoisonError::into_inner);
        totals.in_runner.then_some(UNCOUNTED)
    }
}

impl Drop for PipelineTiming {
    fn drop(&mut self) {
        self.timings
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|totals| !Arc::ptr_eq(totals, &self.totals));
    }
}

//! The scripted provider runtime and clocks the compaction tests build on: a
//! runtime that plays scripted runs in order, the events a run yields and
//! wall clocks. No test calls a real model.

use std::{cell::RefCell, collections::VecDeque, rc::Rc};

use demi_shared_types::{Clock, Model, Timestamp, TokenUsage};
use futures_util::{
    StreamExt,
    future::LocalBoxFuture,
    stream::{self, LocalBoxStream},
};

use crate::{
    Ahead, InferenceRequest, ProviderEvent, ProviderFailure, ProviderRun, ProviderRuntime,
    RequestLimits, ToolCall, ToolResultKinds,
};

/// Builders of the events a scripted run yields.
pub mod event {
    use super::*;

    pub fn text(text: &str) -> ProviderEvent {
        ProviderEvent::TextDelta(text.to_owned())
    }

    pub fn tool_call(
        tool_use_id: &str,
        tool_name: &str,
        input: serde_json::Value,
    ) -> ProviderEvent {
        ProviderEvent::ToolCall(ToolCall {
            tool_use_id: tool_use_id.to_owned(),
            tool_name: tool_name.to_owned(),
            input,
        })
    }

    /// A response whose usage is `input_tokens` in and `output_tokens` out.
    pub fn response(input_tokens: u64, output_tokens: u64) -> ProviderEvent {
        ProviderEvent::Response(Some(TokenUsage {
            input_tokens,
            output_tokens,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
        }))
    }

    /// A failure with a code and no diagnostics.
    pub fn error(message: &str, code: Option<crate::ErrorCode>) -> ProviderEvent {
        ProviderEvent::Error(ProviderFailure {
            message: message.to_owned(),
            code,
            diagnostics: None,
            retry_after: None,
        })
    }
}

/// The events a scripted run makes of its request.
pub type RequestEvents = Box<dyn FnOnce(&InferenceRequest) -> Vec<ProviderEvent>>;

/// The stream a scripted run opens for its request.
pub type RequestStream =
    Box<dyn FnOnce(&InferenceRequest) -> LocalBoxStream<'static, ProviderEvent>>;

/// One scripted run.
pub enum Turn {
    /// These events, in order.
    Events(Vec<ProviderEvent>),
    /// The events a function of the request returns, so that a test can
    /// answer what the request carries.
    Respond(RequestEvents),
    /// Any stream, for a test that controls timing, such as a run that
    /// never ends until it is cancelled.
    Stream(RequestStream),
}

impl Turn {
    /// A run that yields nothing and ends only when it is cancelled.
    pub fn pending() -> Self {
        Self::Stream(Box::new(|_| stream::pending().boxed_local()))
    }
}

/// A runtime that plays scripted runs in order, one per `run`.
///
/// A runtime and its [`fresh`](ProviderRuntime::fresh) copies share one script,
/// because a test describes one ordered scenario across a session and its
/// compaction copy. A run honors its request's cancellation like a real one. A
/// run beyond the script panics: the test did not script it.
#[derive(Clone)]
pub struct ScriptedRuntime {
    script: Rc<RefCell<Script>>,
}

struct Script {
    turns: VecDeque<Turn>,
    requests: Vec<InferenceRequest>,
    closes: usize,
    limits: RequestLimits,
}

impl ScriptedRuntime {
    pub fn new(turns: impl IntoIterator<Item = Turn>) -> Self {
        Self {
            script: Rc::new(RefCell::new(Script {
                turns: turns.into_iter().collect(),
                requests: Vec::new(),
                closes: 0,
                limits: RequestLimits {
                    tool_results: ToolResultKinds::ALL,
                    ..RequestLimits::default()
                },
            })),
        }
    }

    /// The runtime of a vendor that takes requests within `limits`, for
    /// every model; by default it refuses nothing for its size and carries
    /// every kind of medium in a tool result.
    #[must_use]
    pub fn with_limits(self, limits: RequestLimits) -> Self {
        self.script.borrow_mut().limits = limits;
        self
    }

    /// Every request run so far, in order.
    pub fn requests(&self) -> Vec<InferenceRequest> {
        self.script.borrow().requests.clone()
    }

    /// The scripted runs not yet played.
    pub fn remaining(&self) -> usize {
        self.script.borrow().turns.len()
    }

    /// How often a runtime of this script was closed.
    pub fn closes(&self) -> usize {
        self.script.borrow().closes
    }
}

impl ProviderRuntime for ScriptedRuntime {
    fn run(&mut self, request: InferenceRequest) -> ProviderRun<'_> {
        let (turn, number) = {
            let mut script = self.script.borrow_mut();
            script.requests.push(request.clone());
            (script.turns.pop_front(), script.requests.len())
        };
        let Some(turn) = turn else {
            panic!("ScriptedRuntime: no turn scripted for run #{number}");
        };
        let events = match turn {
            Turn::Events(events) => stream::iter(events).boxed_local(),
            Turn::Respond(respond) => stream::iter(respond(&request)).boxed_local(),
            Turn::Stream(open) => open(&request),
        };
        events
            .take_until(request.cancel.cancelled_owned())
            .boxed_local()
    }

    fn fresh(&self) -> Box<dyn ProviderRuntime> {
        Box::new(self.clone())
    }

    fn close(&mut self) -> LocalBoxFuture<'_, ()> {
        self.script.borrow_mut().closes += 1;
        Box::pin(async {})
    }

    fn prepares_ahead(&self) -> bool {
        true
    }

    fn prepare(&mut self, _request: InferenceRequest, _ahead: Ahead) -> LocalBoxFuture<'_, ()> {
        Box::pin(async {})
    }

    fn request_limits(&self, _model: &Model) -> RequestLimits {
        self.script.borrow().limits
    }
}

/// A wall clock that always reads the same moment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixedClock(pub Timestamp);

impl Clock for FixedClock {
    fn now(&self) -> Timestamp {
        self.0
    }
}

/// A wall clock that moves with Tokio's, so that a test on the paused clock
/// moves the times records carry too: `start` plus the Tokio time elapsed
/// since the clock was made.
#[derive(Debug)]
pub struct TokioClock {
    start: Timestamp,
    origin: tokio::time::Instant,
}

impl TokioClock {
    pub fn new(start: Timestamp) -> Self {
        Self {
            start,
            origin: tokio::time::Instant::now(),
        }
    }
}

impl Clock for TokioClock {
    fn now(&self) -> Timestamp {
        let elapsed = i64::try_from(self.origin.elapsed().as_millis()).unwrap_or(i64::MAX);
        Timestamp::from_millisecond(self.start.as_millisecond().saturating_add(elapsed))
            .expect("a test's time stays in range")
    }
}

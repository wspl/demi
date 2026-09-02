//! Per-process state reachable from every primitive through the context's
//! user data.
//!
//! `active` counts the work that keeps the process alive: in-flight IO
//! operations, live timers and child waits. When the entry module has
//! finished and the count is zero the event loop exits, exactly like Node's
//! reference counting of handles — signal handlers do not count.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::time::Instant;

use rquickjs::{Ctx, Function, JsLifetime, Persistent, Value};
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::Notify;

use crate::handles::Handles;

pub struct State {
    pub start: Instant,
    pub handles: RefCell<Handles>,
    active: Cell<usize>,
    pub quiesced: Notify,
    /// Signals the process receives are forwarded here and dispatched from
    /// the event loop to the handlers registered through `onSignal`.
    pub signal_tx: UnboundedSender<&'static str>,
    pub signal_handlers: RefCell<HashMap<&'static str, Persistent<Function<'static>>>>,
    /// Rejected promises that had no handler when they rejected, keyed by
    /// the promise pointer; removed again if a handler is attached before
    /// the check runs.
    pub unhandled: RefCell<HashMap<usize, Persistent<Value<'static>>>>,
}

unsafe impl<'js> JsLifetime<'js> for State {
    type Changed<'to> = State;
}

impl State {
    pub fn new(signal_tx: UnboundedSender<&'static str>) -> Self {
        State {
            start: Instant::now(),
            handles: RefCell::new(Handles::new()),
            active: Cell::new(0),
            quiesced: Notify::new(),
            signal_tx,
            signal_handlers: RefCell::new(HashMap::new()),
            unhandled: RefCell::new(HashMap::new()),
        }
    }

    pub fn active(&self) -> usize {
        self.active.get()
    }
}

/// The state is stored once at startup; a missing entry is a programming error.
pub fn state<'a, 'js>(ctx: &'a Ctx<'js>) -> rquickjs::runtime::UserDataGuard<'a, State> {
    ctx.userdata::<State>().expect("shell state is installed at startup")
}

/// Holds one unit of liveness for as long as it exists.
pub struct Activity<'js> {
    ctx: Ctx<'js>,
}

impl<'js> Activity<'js> {
    pub fn begin(ctx: &Ctx<'js>) -> Self {
        let st = state(ctx);
        st.active.set(st.active.get() + 1);
        Activity { ctx: ctx.clone() }
    }
}

impl<'js> Drop for Activity<'js> {
    fn drop(&mut self) {
        let st = state(&self.ctx);
        let n = st.active.get() - 1;
        st.active.set(n);
        if n == 0 {
            st.quiesced.notify_one();
        }
    }
}

//! `setTimeout` / `setInterval` and their clears.
//!
//! One queue ordered by (deadline, sequence) and one driver future that runs
//! only while the queue is non-empty, so timers fire in order and an empty
//! queue never keeps the process alive.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;
use std::time::{Duration, Instant};

use rquickjs::function::{Args, Opt, Rest};
use rquickjs::{CatchResultExt, Ctx, Function, Result, Value};
use tokio::sync::Notify;

use crate::event_loop::report_uncaught;
use crate::state::Activity;

struct Timer<'js> {
    id: u32,
    callback: Function<'js>,
    args: Vec<Value<'js>>,
    period: Option<Duration>,
}

struct Queue<'js> {
    by_time: BTreeMap<(Instant, u64), Timer<'js>>,
    by_id: HashMap<u32, (Instant, u64)>,
    seq: u64,
    next_id: u32,
    driver_running: bool,
    wake: Rc<Notify>,
}

impl<'js> Queue<'js> {
    fn add(&mut self, delay: f64, callback: Function<'js>, args: Vec<Value<'js>>, period: Option<Duration>) -> u32 {
        let delay = if delay.is_finite() && delay > 0.0 { delay } else { 0.0 };
        let when = Instant::now() + Duration::from_secs_f64(delay / 1000.0);
        let id = self.next_id;
        self.next_id += 1;
        self.insert(when, Timer { id, callback, args, period });
        id
    }

    fn insert(&mut self, when: Instant, timer: Timer<'js>) {
        self.seq += 1;
        let key = (when, self.seq);
        self.by_id.insert(timer.id, key);
        self.by_time.insert(key, timer);
        self.wake.notify_one();
    }

    fn clear(&mut self, id: u32) {
        if let Some(key) = self.by_id.remove(&id) {
            self.by_time.remove(&key);
            self.wake.notify_one();
        }
    }

    fn pop_due(&mut self) -> Option<Timer<'js>> {
        let key = *self.by_time.keys().next()?;
        if key.0 > Instant::now() {
            return None;
        }
        let timer = self.by_time.remove(&key)?;
        self.by_id.remove(&timer.id);
        Some(timer)
    }
}

type Shared<'js> = Rc<RefCell<Queue<'js>>>;

pub fn install<'js>(ctx: &Ctx<'js>) -> Result<()> {
    let queue: Shared<'js> = Rc::new(RefCell::new(Queue {
        by_time: BTreeMap::new(),
        by_id: HashMap::new(),
        seq: 0,
        next_id: 1,
        driver_running: false,
        wake: Rc::new(Notify::new()),
    }));
    let globals = ctx.globals();
    let q = queue.clone();
    globals.set(
        "setTimeout",
        Function::new(ctx.clone(), move |ctx: Ctx<'js>, cb: Function<'js>, delay: Opt<f64>, args: Rest<Value<'js>>| {
            schedule(&ctx, &q, cb, delay.0.unwrap_or(0.0), args.0, false)
        })?,
    )?;
    let q = queue.clone();
    globals.set(
        "setInterval",
        Function::new(ctx.clone(), move |ctx: Ctx<'js>, cb: Function<'js>, delay: Opt<f64>, args: Rest<Value<'js>>| {
            schedule(&ctx, &q, cb, delay.0.unwrap_or(0.0), args.0, true)
        })?,
    )?;
    let q = queue.clone();
    let clear = Function::new(ctx.clone(), move |id: Opt<Value<'js>>| {
        if let Some(id) = id.0.and_then(|v| v.as_number()) {
            q.borrow_mut().clear(id as u32);
        }
    })?;
    globals.set("clearTimeout", clear.clone())?;
    globals.set("clearInterval", clear)?;
    Ok(())
}

fn schedule<'js>(
    ctx: &Ctx<'js>,
    queue: &Shared<'js>,
    callback: Function<'js>,
    delay: f64,
    args: Vec<Value<'js>>,
    repeat: bool,
) -> u32 {
    let period = repeat.then(|| Duration::from_secs_f64(delay.max(1.0) / 1000.0));
    let mut q = queue.borrow_mut();
    let id = q.add(delay, callback, args, period);
    if !q.driver_running {
        q.driver_running = true;
        drop(q);
        ctx.spawn(drive(ctx.clone(), queue.clone()));
    }
    id
}

async fn drive<'js>(ctx: Ctx<'js>, queue: Shared<'js>) {
    let activity = Activity::begin(&ctx);
    loop {
        let (next, wake) = {
            let q = queue.borrow();
            (q.by_time.keys().next().map(|k| k.0), q.wake.clone())
        };
        let Some(when) = next else {
            queue.borrow_mut().driver_running = false;
            drop(activity);
            return;
        };
        tokio::select! {
            _ = tokio::time::sleep_until(tokio::time::Instant::from_std(when)) => {}
            _ = wake.notified() => continue,
        }
        loop {
            // The borrow must end before the callback runs: it may schedule
            // or clear timers itself.
            let due = queue.borrow_mut().pop_due();
            let Some(timer) = due else { break };
            if let Some(period) = timer.period {
                let again = Timer { id: timer.id, callback: timer.callback.clone(), args: timer.args.clone(), period: Some(period) };
                queue.borrow_mut().insert(Instant::now() + period, again);
            }
            let call = || -> Result<()> {
                let mut a = Args::new(ctx.clone(), timer.args.len());
                a.push_args(timer.args.iter().cloned())?;
                timer.callback.call_arg::<()>(a)
            };
            if let Err(e) = call().catch(&ctx) {
                report_uncaught(&e);
                std::process::exit(1);
            }
        }
    }
}

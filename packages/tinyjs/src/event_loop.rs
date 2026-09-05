//! The event loop: a current-thread tokio runtime driving one QuickJS
//! context.
//!
//! The whole program runs inside a single `async_with` so the runtime lock
//! is held once. Inside it the loop selects between the entry module's
//! completion, incoming signals and quiescence; primitives are futures the
//! QuickJS scheduler polls in between.

use rquickjs::{async_with, AsyncContext, AsyncRuntime, CatchResultExt, CaughtError, Ctx, Function, Persistent, Value};
use tokio::sync::mpsc;

use crate::state::{state, State};
use crate::payload::Payload;
use crate::{globals, loader};

pub fn run(payload: Payload, argv: Vec<String>) -> i32 {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    rt.block_on(run_async(payload, argv))
}

async fn run_async(payload: Payload, argv: Vec<String>) -> i32 {
    let entry_name = payload.entry_name();
    let entry = match payload.entry() {
        Ok(entry) => entry,
        Err(e) => {
            eprintln!("tinyjs: cannot read entry '{entry_name}': {e}");
            return 2;
        }
    };
    let js = AsyncRuntime::new().expect("QuickJS runtime");
    js.set_loader(loader::ShellResolver, loader::ShellLoader::new(payload)).await;
    js.set_host_promise_rejection_tracker(Some(Box::new(track_rejection)))
        .await;
    let context = AsyncContext::full(&js).await.expect("QuickJS context");
    let (signal_tx, mut signal_rx) = mpsc::unbounded_channel::<&'static str>();

    async_with!(context => |ctx| {
        ctx.store_userdata(State::new(signal_tx, argv)).expect("store state");
        if let Err(e) = globals::install(&ctx).catch(&ctx) {
            report_uncaught(&e);
            return 1;
        }
        let entry = match loader::evaluate_entry(&ctx, &entry_name, entry).catch(&ctx) {
            Ok(p) => p.into_future::<()>(),
            Err(e) => {
                report_uncaught(&e);
                return 1;
            }
        };
        tokio::pin!(entry);
        let mut entry_done = false;
        loop {
            tokio::select! {
                r = &mut entry, if !entry_done => {
                    entry_done = true;
                    if let Err(e) = r.catch(&ctx) {
                        report_uncaught(&e);
                        return 1;
                    }
                    if quiescent(&ctx).await {
                        return 0;
                    }
                }
                Some(name) = signal_rx.recv() => {
                    dispatch_signal(&ctx, name);
                }
                _ = wait_quiesced(&ctx), if entry_done => {
                    if quiescent(&ctx).await {
                        return 0;
                    }
                }
            }
        }
    })
    .await
}

async fn wait_quiesced(ctx: &Ctx<'_>) {
    // The guard only blocks inserting new user data, so holding it across
    // the await is harmless.
    let st = state(ctx);
    if st.active() == 0 {
        return;
    }
    st.quiesced.notified().await
}

/// Lets the scheduler drain pending jobs once, then reports whether nothing
/// is left that could produce more work.
async fn quiescent(ctx: &Ctx<'_>) -> bool {
    tokio::task::yield_now().await;
    state(ctx).active() == 0
}

fn dispatch_signal(ctx: &Ctx<'_>, name: &'static str) {
    let handler = state(ctx).signal_handlers.borrow().get(name).cloned();
    if let Some(handler) = handler {
        let call = || -> rquickjs::Result<()> {
            let f: Function<'_> = handler.restore(ctx)?;
            f.call::<_, ()>((name,))
        };
        if let Err(e) = call().catch(ctx) {
            report_uncaught(&e);
            std::process::exit(1);
        }
    }
}

pub fn report_uncaught(err: &CaughtError<'_>) {
    eprintln!("Uncaught {err}");
}

fn promise_key(promise: &Value<'_>) -> usize {
    // SAFETY: a promise is an object, so the pointer member of the union is
    // the initialised one.
    unsafe { promise.as_raw().u.ptr as usize }
}

fn track_rejection<'js>(ctx: Ctx<'js>, promise: Value<'js>, reason: Value<'js>, is_handled: bool) {
    let key = promise_key(&promise);
    let st = state(&ctx);
    if is_handled {
        st.unhandled.borrow_mut().remove(&key);
        return;
    }
    st.unhandled
        .borrow_mut()
        .insert(key, Persistent::save(&ctx, reason));
    drop(st);
    let check_ctx = ctx.clone();
    ctx.spawn(async move {
        tokio::task::yield_now().await;
        let reason = state(&check_ctx).unhandled.borrow_mut().remove(&key);
        if let Some(reason) = reason {
            let reason: Option<Value<'_>> = reason.restore(&check_ctx).ok();
            let text = reason
                .map(|v| describe(&check_ctx, v))
                .unwrap_or_else(|| "unknown reason".into());
            eprintln!("Uncaught (in promise) {text}");
            std::process::exit(1);
        }
    });
}

/// A printable form of a thrown value: message and stack for errors, the
/// string form otherwise.
pub fn describe<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> String {
    if let Some(ex) = value.clone().into_object().and_then(rquickjs::Exception::from_object) {
        return format!("{ex}");
    }
    ctx.json_stringify(value.clone())
        .ok()
        .flatten()
        .and_then(|s| s.to_string().ok())
        .or_else(|| value.get::<rquickjs::String>().ok().and_then(|s| s.to_string().ok()))
        .unwrap_or_else(|| "<value>".into())
}

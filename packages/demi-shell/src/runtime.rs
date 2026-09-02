//! `demishell:runtime`: the process itself — arguments, environment, cwd,
//! exit, signals, the standard streams and the host identity.

use std::ffi::CStr;

use rquickjs::function::Func;
use rquickjs::module::{Declarations, Exports, ModuleDef};
use rquickjs::{Array, Ctx, Function, Object, Persistent, Result};
use tokio::signal::unix::{signal, SignalKind};

use crate::error::{invalid, throw_io};
use crate::state::state;

/// Bumped when the shell changes in any way the runner `hello` reports.
pub const VERSION: u32 = 1;
/// Bumped when the `demishell:*` surface changes incompatibly; host-shell
/// checks it at start.
pub const ABI: u32 = 1;

pub struct RuntimeModule;

impl ModuleDef for RuntimeModule {
    fn declare(decl: &Declarations<'_>) -> Result<()> {
        for name in [
            "argv", "env", "cwd", "chdir", "exit", "onSignal", "stdin", "stdout", "stderr", "pid",
            "identity", "version", "abi", "openHandles",
        ] {
            decl.declare(name)?;
        }
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
        let argv = Array::new(ctx.clone())?;
        for (i, a) in std::env::args_os().enumerate() {
            argv.set(i, a.to_string_lossy().into_owned())?;
        }
        exports.export("argv", argv)?;
        let env = Object::new(ctx.clone())?;
        for (k, v) in std::env::vars_os() {
            env.set(k.to_string_lossy().into_owned(), v.to_string_lossy().into_owned())?;
        }
        let freeze: Function = ctx.globals().get::<_, Object>("Object")?.get("freeze")?;
        freeze.call::<_, ()>((env.clone(),))?;
        exports.export("env", env)?;
        exports.export("cwd", Func::from(cwd))?;
        exports.export("chdir", Func::from(chdir))?;
        exports.export("exit", Func::from(exit))?;
        exports.export("onSignal", Func::from(on_signal))?;
        exports.export("stdin", 0)?;
        exports.export("stdout", 1)?;
        exports.export("stderr", 2)?;
        exports.export("pid", std::process::id())?;
        exports.export("identity", identity(ctx)?)?;
        exports.export("version", VERSION)?;
        exports.export("abi", ABI)?;
        exports.export("openHandles", Func::from(open_handles))?;
        Ok(())
    }
}

/// Handles open above the standard streams; a leak is countable in tests.
fn open_handles(ctx: Ctx<'_>) -> usize {
    state(&ctx).handles.borrow().open_count()
}

fn cwd(ctx: Ctx<'_>) -> Result<String> {
    std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .map_err(|e| throw_io(&ctx, e, "getcwd", None))
}

fn chdir(ctx: Ctx<'_>, path: String) -> Result<()> {
    std::env::set_current_dir(&path).map_err(|e| throw_io(&ctx, e, "chdir", Some(&path)))
}

fn exit(code: rquickjs::function::Opt<i32>) {
    std::process::exit(code.0.unwrap_or(0));
}

const SIGNALS: &[(&str, fn() -> SignalKind)] = &[
    ("SIGTERM", SignalKind::terminate),
    ("SIGINT", SignalKind::interrupt),
    ("SIGHUP", SignalKind::hangup),
    ("SIGUSR1", SignalKind::user_defined1),
    ("SIGUSR2", SignalKind::user_defined2),
];

/// Registers the handler for a signal. The first registration installs the
/// OS handler; a later call replaces the JS handler only. Handlers do not
/// keep the process alive.
fn on_signal<'js>(ctx: Ctx<'js>, name: String, handler: Function<'js>) -> Result<()> {
    let Some((static_name, kind)) = SIGNALS.iter().find(|(n, _)| *n == name) else {
        return Err(invalid(&ctx, "signal", &format!("unsupported signal '{name}'")));
    };
    let st = state(&ctx);
    let first = st
        .signal_handlers
        .borrow_mut()
        .insert(static_name, Persistent::save(&ctx, handler))
        .is_none();
    if first {
        let mut stream = signal(kind()).map_err(|e| throw_io(&ctx, e, "sigaction", None))?;
        let tx = st.signal_tx.clone();
        tokio::spawn(async move {
            while stream.recv().await.is_some() {
                if tx.send(static_name).is_err() {
                    break;
                }
            }
        });
    }
    Ok(())
}

fn identity<'js>(ctx: &Ctx<'js>) -> Result<Object<'js>> {
    let obj = Object::new(ctx.clone())?;
    // SAFETY: getuid/getgid/gethostname/getpwuid are plain libc calls; the
    // passwd record is only read before any other libc call.
    let (uid, gid) = unsafe { (libc::getuid(), libc::getgid()) };
    let mut buf = [0u8; 256];
    let hostname = unsafe {
        if libc::gethostname(buf.as_mut_ptr() as *mut _, buf.len()) == 0 {
            CStr::from_ptr(buf.as_ptr() as *const _).to_string_lossy().into_owned()
        } else {
            String::new()
        }
    };
    let home_dir = std::env::var("HOME").ok().unwrap_or_else(|| unsafe {
        let pw = libc::getpwuid(uid);
        if pw.is_null() { String::new() } else { CStr::from_ptr((*pw).pw_dir).to_string_lossy().into_owned() }
    });
    obj.set("uid", uid)?;
    obj.set("gid", gid)?;
    obj.set("hostname", hostname)?;
    obj.set("homeDir", home_dir)?;
    Ok(obj)
}

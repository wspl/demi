//! `tinyjs:process`: spawn with pipes, wait, kill, uid/gid, process
//! groups and the tee with a bounded view.
//!
//! Children are spawned with `std::process::Command` and reaped by the
//! tinyjs's own SIGCHLD loop (`waitpid(-1)`), which also reaps adopted
//! orphans when tinyjs is PID 1. Stdio pipes are handed to tokio and
//! enter the handle table as streams.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::io;
use std::os::fd::OwnedFd;
use std::os::unix::process::CommandExt;
use std::pin::Pin;
use std::process::Stdio;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

use rquickjs::function::{Async, Func, Opt};
use rquickjs::module::{Declarations, Exports, ModuleDef};
use rquickjs::{Ctx, Object, Result, Value};
use tokio::io::{AsyncRead, AsyncReadExt, ReadBuf};
use tokio::net::unix::pipe;
use tokio::signal::unix::{signal, SignalKind};

use crate::error::{invalid, throw_errno, throw_io};
use crate::handles::{Resource, Stream};
use crate::state::{state, Activity};

pub struct ProcessModule;

impl ModuleDef for ProcessModule {
    fn declare(decl: &Declarations<'_>) -> Result<()> {
        decl.declare("spawn")?.declare("wait")?.declare("kill")?;
        Ok(())
    }

    fn evaluate<'js>(_ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
        exports.export("spawn", Func::from(Async(spawn)))?;
        exports.export("wait", Func::from(Async(wait)))?;
        exports.export("kill", Func::from(kill))?;
        Ok(())
    }
}

/// What `wait` needs about one child.
pub struct ChildSlot {
    status: Option<i32>,
    exited: Rc<tokio::sync::Notify>,
    tee: Option<Rc<TeeOutcome>>,
}

#[derive(Default)]
pub struct Children {
    map: HashMap<i32, ChildSlot>,
    reaper_running: bool,
}

const SIGNALS: &[(&str, i32)] = &[
    ("SIGHUP", libc::SIGHUP),
    ("SIGINT", libc::SIGINT),
    ("SIGQUIT", libc::SIGQUIT),
    ("SIGILL", libc::SIGILL),
    ("SIGTRAP", libc::SIGTRAP),
    ("SIGABRT", libc::SIGABRT),
    ("SIGBUS", libc::SIGBUS),
    ("SIGFPE", libc::SIGFPE),
    ("SIGKILL", libc::SIGKILL),
    ("SIGUSR1", libc::SIGUSR1),
    ("SIGSEGV", libc::SIGSEGV),
    ("SIGUSR2", libc::SIGUSR2),
    ("SIGPIPE", libc::SIGPIPE),
    ("SIGALRM", libc::SIGALRM),
    ("SIGTERM", libc::SIGTERM),
    ("SIGCHLD", libc::SIGCHLD),
    ("SIGCONT", libc::SIGCONT),
    ("SIGSTOP", libc::SIGSTOP),
    ("SIGTSTP", libc::SIGTSTP),
    ("SIGTTIN", libc::SIGTTIN),
    ("SIGTTOU", libc::SIGTTOU),
    ("SIGWINCH", libc::SIGWINCH),
];

fn signal_number(name: &str) -> Option<i32> {
    SIGNALS.iter().find(|(n, _)| *n == name).map(|(_, v)| *v)
}

fn signal_name(number: i32) -> String {
    SIGNALS
        .iter()
        .find(|(_, v)| *v == number)
        .map(|(n, _)| n.to_string())
        .unwrap_or_else(|| format!("SIG{number}"))
}

// --- the tee view -----------------------------------------------------------------

/// The first `limit` bytes of a teed stream, readable through a handle;
/// ends once the limit is reached or the stream closes.
struct View {
    chunks: VecDeque<Vec<u8>>,
    offset: usize,
    remaining: usize,
    ended: bool,
    waker: Option<Waker>,
}

struct ViewReader(Rc<RefCell<View>>);

impl AsyncRead for ViewReader {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        let mut view = self.0.borrow_mut();
        let offset = view.offset;
        if let Some(front) = view.chunks.front() {
            let available = &front[offset..];
            let n = available.len().min(buf.remaining());
            buf.put_slice(&available[..n]);
            let consumed = offset + n >= front.len();
            view.offset = if consumed { 0 } else { offset + n };
            if consumed {
                view.chunks.pop_front();
            }
            return Poll::Ready(Ok(()));
        }
        if view.ended {
            return Poll::Ready(Ok(()));
        }
        view.waker = Some(cx.waker().clone());
        Poll::Pending
    }
}

impl View {
    fn push(&mut self, data: &[u8]) {
        if self.remaining > 0 {
            let n = data.len().min(self.remaining);
            self.chunks.push_back(data[..n].to_vec());
            self.remaining -= n;
        }
        if self.remaining == 0 {
            self.ended = true;
        }
        if let Some(w) = self.waker.take() {
            w.wake();
        }
    }

    fn end(&mut self) {
        self.ended = true;
        if let Some(w) = self.waker.take() {
            w.wake();
        }
    }
}

#[derive(Default)]
pub struct TeeOutcome {
    stdout: Cell<Option<u64>>,
    stderr: Cell<Option<u64>>,
    error: RefCell<Option<io::Error>>,
    done: tokio::sync::Notify,
}

/// Drains one child stream into its file and the bounded view.
///
/// The file is written synchronously on the loop thread: a page-cache
/// write of one chunk is microseconds, while handing each chunk to the
/// blocking pool costs a thread wake-up, which under nested
/// virtualisation is what halved the throughput.
async fn tee<'js>(
    ctx: Ctx<'js>,
    mut source: pipe::Receiver,
    mut file: std::fs::File,
    view: Rc<RefCell<View>>,
    outcome: Rc<TeeOutcome>,
    is_stdout: bool,
) {
    use std::io::Write;
    let _activity = Activity::begin(&ctx);
    let mut buf = vec![0u8; 256 * 1024];
    let mut total: u64 = 0;
    let mut failed = false;
    loop {
        let n = match source.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) => {
                *outcome.error.borrow_mut() = Some(e);
                break;
            }
        };
        total += n as u64;
        if !failed {
            if let Err(e) = file.write_all(&buf[..n]) {
                // Keep draining so the child never blocks; report at wait.
                *outcome.error.borrow_mut() = Some(e);
                failed = true;
            }
        }
        view.borrow_mut().push(&buf[..n]);
    }
    view.borrow_mut().end();
    if is_stdout { outcome.stdout.set(Some(total)) } else { outcome.stderr.set(Some(total)) }
    outcome.done.notify_waiters();
}

// --- the reaper -------------------------------------------------------------------

fn ensure_reaper<'js>(ctx: &Ctx<'js>) -> Result<()> {
    let st = state(ctx);
    if st.children.borrow().reaper_running {
        return Ok(());
    }
    // Registered before the first spawn so no SIGCHLD can be missed.
    let mut stream = signal(SignalKind::child()).map_err(|e| throw_io(ctx, e, "sigaction", None))?;
    st.children.borrow_mut().reaper_running = true;
    let ctx2 = ctx.clone();
    ctx.spawn(async move {
        while stream.recv().await.is_some() {
            reap_all(&ctx2);
        }
    });
    Ok(())
}

/// Reaps every exited child. Unknown pids are adopted orphans (PID 1) and
/// are simply released.
fn reap_all(ctx: &Ctx<'_>) {
    loop {
        let mut status: libc::c_int = 0;
        // SAFETY: plain waitpid with a valid status pointer.
        let pid = unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG) };
        if pid <= 0 {
            break;
        }
        let st = state(ctx);
        let mut children = st.children.borrow_mut();
        if let Some(slot) = children.map.get_mut(&pid) {
            slot.status = Some(status);
            slot.exited.notify_waiters();
        }
    }
}

// --- spawn / wait / kill ----------------------------------------------------------

async fn spawn<'js>(ctx: Ctx<'js>, options: Object<'js>) -> Result<Object<'js>> {
    let command: String = options.get("command")?;
    let args: Vec<String> = options.get::<_, Option<Vec<String>>>("args")?.unwrap_or_default();
    let cwd: Option<String> = options.get("cwd")?;
    let env: Option<Object> = options.get("env")?;
    let stdin_mode: String = options.get::<_, Option<String>>("stdin")?.unwrap_or_else(|| "null".into());
    let uid: Option<u32> = options.get("uid")?;
    let gid: Option<u32> = options.get("gid")?;
    let process_group: bool = options.get::<_, Option<bool>>("processGroup")?.unwrap_or(false);
    let tee_opts: Option<Object> = options.get("tee")?;

    let mut cmd = std::process::Command::new(&command);
    cmd.args(&args);
    if let Some(cwd) = &cwd {
        cmd.current_dir(cwd);
    }
    cmd.env_clear();
    if let Some(env) = env {
        for r in env.props::<String, String>() {
            let (k, v) = r?;
            cmd.env(k, v);
        }
    }
    match stdin_mode.as_str() {
        "pipe" => cmd.stdin(Stdio::piped()),
        "null" => cmd.stdin(Stdio::null()),
        other => return Err(invalid(&ctx, "spawn", &format!("stdin must be 'pipe' or 'null', got '{other}'"))),
    };
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    if let Some(uid) = uid {
        cmd.uid(uid);
    }
    if let Some(gid) = gid {
        cmd.gid(gid);
    }
    if process_group {
        cmd.process_group(0);
    }

    // Tee files are opened first so their errors carry their path.
    let tee_files = match tee_opts {
        Some(t) => {
            let stdout_path: String = t.get("stdoutPath")?;
            let stderr_path: String = t.get("stderrPath")?;
            let limit: u32 = t.get::<_, Option<u32>>("viewLimit")?.unwrap_or(0);
            let open = |p: &str| -> Result<std::fs::File> {
                std::fs::OpenOptions::new().write(true).create(true).truncate(true).open(p)
                    .map_err(|e| throw_io(&ctx, e, "open", Some(p)))
            };
            Some((open(&stdout_path)?, open(&stderr_path)?, limit as usize))
        }
        None => None,
    };

    ensure_reaper(&ctx)?;
    let mut child = cmd.spawn().map_err(|e| throw_io(&ctx, e, "spawn", Some(&command)))?;
    let pid = child.id() as i32;
    let to_receiver = |f: Option<std::process::ChildStdout>, e: Option<std::process::ChildStderr>| -> io::Result<(pipe::Receiver, pipe::Receiver)> {
        let out = pipe::Receiver::from_owned_fd(OwnedFd::from(f.expect("stdout piped")))?;
        let err = pipe::Receiver::from_owned_fd(OwnedFd::from(e.expect("stderr piped")))?;
        Ok((out, err))
    };
    let (stdout, stderr) = to_receiver(child.stdout.take(), child.stderr.take())
        .map_err(|e| throw_io(&ctx, e, "pipe", None))?;
    let stdin = match child.stdin.take() {
        Some(s) => Some(pipe::Sender::from_owned_fd(OwnedFd::from(s)).map_err(|e| throw_io(&ctx, e, "pipe", None))?),
        None => None,
    };

    let st = state(&ctx);
    let mut handles = st.handles.borrow_mut();
    let stdin_fd = stdin.map(|s| handles.insert(Resource::Stream(Stream::new(None, Some(Box::new(s))))));
    let (stdout_fd, stderr_fd, outcome) = match tee_files {
        None => (
            handles.insert(Resource::Stream(Stream::new(Some(Box::new(stdout)), None))),
            handles.insert(Resource::Stream(Stream::new(Some(Box::new(stderr)), None))),
            None,
        ),
        Some((out_file, err_file, limit)) => {
            let outcome = Rc::new(TeeOutcome::default());
            let new_view = || Rc::new(RefCell::new(View { chunks: VecDeque::new(), offset: 0, remaining: limit, ended: limit == 0, waker: None }));
            let (out_view, err_view) = (new_view(), new_view());
            ctx.spawn(tee(ctx.clone(), stdout, out_file, out_view.clone(), outcome.clone(), true));
            ctx.spawn(tee(ctx.clone(), stderr, err_file, err_view.clone(), outcome.clone(), false));
            (
                handles.insert(Resource::Stream(Stream::new(Some(Box::new(ViewReader(out_view))), None))),
                handles.insert(Resource::Stream(Stream::new(Some(Box::new(ViewReader(err_view))), None))),
                Some(outcome),
            )
        }
    };
    drop(handles);
    st.children.borrow_mut().map.insert(pid, ChildSlot { status: None, exited: Rc::new(tokio::sync::Notify::new()), tee: outcome });
    drop(st);
    // The std Child is dropped without wait; our reaper owns the status.
    std::mem::forget(child);

    let result = Object::new(ctx.clone())?;
    result.set("pid", pid)?;
    result.set("stdin", match stdin_fd { Some(fd) => Value::new_int(ctx.clone(), fd), None => Value::new_null(ctx.clone()) })?;
    result.set("stdout", stdout_fd)?;
    result.set("stderr", stderr_fd)?;
    Ok(result)
}

async fn wait<'js>(ctx: Ctx<'js>, pid: i32) -> Result<Object<'js>> {
    let _activity = Activity::begin(&ctx);
    let (exited, tee) = {
        let st = state(&ctx);
        let children = st.children.borrow();
        match children.map.get(&pid) {
            None => return Err(throw_errno(&ctx, libc::ECHILD, "wait", None)),
            Some(slot) => (slot.exited.clone(), slot.tee.clone()),
        }
    };
    let status = loop {
        let status = state(&ctx).children.borrow().map.get(&pid).and_then(|s| s.status);
        if let Some(s) = status {
            break s;
        }
        exited.notified().await;
    };
    let mut counts = None;
    if let Some(outcome) = tee {
        loop {
            if let (Some(o), Some(e)) = (outcome.stdout.get(), outcome.stderr.get()) {
                counts = Some((o, e));
                break;
            }
            outcome.done.notified().await;
        }
        if let Some(err) = outcome.error.borrow_mut().take() {
            state(&ctx).children.borrow_mut().map.remove(&pid);
            return Err(throw_io(&ctx, err, "tee", None));
        }
    }
    state(&ctx).children.borrow_mut().map.remove(&pid);

    let result = Object::new(ctx.clone())?;
    if libc::WIFEXITED(status) {
        result.set("code", libc::WEXITSTATUS(status))?;
    } else {
        result.set("code", Value::new_null(ctx.clone()))?;
        if libc::WIFSIGNALED(status) {
            result.set("signal", signal_name(libc::WTERMSIG(status)))?;
        }
    }
    if let Some((o, e)) = counts {
        result.set("stdoutBytes", o as f64)?;
        result.set("stderrBytes", e as f64)?;
    }
    Ok(result)
}

fn kill(ctx: Ctx<'_>, pid: i32, signal: String, options: Opt<Object<'_>>) -> Result<()> {
    let number = signal_number(&signal).ok_or_else(|| invalid(&ctx, "kill", &format!("unknown signal '{signal}'")))?;
    let group = match options.0 {
        Some(o) => o.get::<_, Option<bool>>("group")?.unwrap_or(false),
        None => false,
    };
    // Only a child of this process can be signalled: as PID 1 in a guest, a
    // stray pid (0, -1, or an unrelated process) would take the machine down.
    if !state(&ctx).children.borrow().map.contains_key(&pid) {
        return Err(throw_io(&ctx, io::Error::from_raw_os_error(libc::ESRCH), "kill", None));
    }
    let target = if group { -pid } else { pid };
    // SAFETY: kill(2) on a pid from our own child table.
    if unsafe { libc::kill(target, number) } != 0 {
        return Err(throw_io(&ctx, io::Error::last_os_error(), "kill", None));
    }
    Ok(())
}

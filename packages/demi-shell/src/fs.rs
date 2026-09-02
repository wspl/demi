//! `demishell:fs`: the `HostFileSystem` method set one to one, with errno
//! fidelity, plus the streaming `open`/`read`/`write`/`close` shared by
//! files, pipes and sockets.
//!
//! Path operations and regular-file IO run on the blocking pool; pipes and
//! sockets are tokio streams. Every large buffer is allocated once at its
//! known size.

use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::sync::Arc;

use rquickjs::function::{Async, Func, Opt};
use rquickjs::module::{Declarations, Exports, ModuleDef};
use rquickjs::{Array, Ctx, Object, Result, TypedArray, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::error::{bad_handle, busy_handle, invalid, throw_errno, throw_io};
use crate::handles::{Reader, Resource, Slot, Writer};
use crate::state::{state, Activity};

pub struct FsModule;

impl ModuleDef for FsModule {
    fn declare(decl: &Declarations<'_>) -> Result<()> {
        for name in [
            "readFile", "writeFile", "stat", "lstat", "readdir", "mkdir", "rmdir", "unlink", "rename",
            "symlink", "link", "readlink", "realpath", "chmod", "utimes", "truncate", "open", "read",
            "write", "close",
        ] {
            decl.declare(name)?;
        }
        Ok(())
    }

    fn evaluate<'js>(_ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
        exports.export("readFile", Func::from(Async(read_file)))?;
        exports.export("writeFile", Func::from(Async(write_file)))?;
        exports.export("stat", Func::from(Async(stat)))?;
        exports.export("lstat", Func::from(Async(lstat)))?;
        exports.export("readdir", Func::from(Async(readdir)))?;
        exports.export("mkdir", Func::from(Async(mkdir)))?;
        exports.export("rmdir", Func::from(Async(rmdir)))?;
        exports.export("unlink", Func::from(Async(unlink)))?;
        exports.export("rename", Func::from(Async(rename)))?;
        exports.export("symlink", Func::from(Async(symlink)))?;
        exports.export("link", Func::from(Async(link)))?;
        exports.export("readlink", Func::from(Async(readlink)))?;
        exports.export("realpath", Func::from(Async(realpath)))?;
        exports.export("chmod", Func::from(Async(chmod)))?;
        exports.export("utimes", Func::from(Async(utimes)))?;
        exports.export("truncate", Func::from(Async(truncate)))?;
        exports.export("open", Func::from(Async(open)))?;
        exports.export("read", Func::from(Async(read)))?;
        exports.export("write", Func::from(Async(write)))?;
        exports.export("close", Func::from(close))?;
        Ok(())
    }
}

/// Runs a blocking file-system call on the pool.
pub async fn blocking<T: Send + 'static>(f: impl FnOnce() -> io::Result<T> + Send + 'static) -> io::Result<T> {
    tokio::task::spawn_blocking(f).await.expect("blocking task panicked")
}

/// Runs a blocking call and maps its error to a `ShellError` with the
/// syscall and path.
async fn path_call<'js, T: Send + 'static>(
    ctx: &Ctx<'js>,
    syscall: &str,
    path: &str,
    f: impl FnOnce() -> io::Result<T> + Send + 'static,
) -> Result<T> {
    let _activity = Activity::begin(ctx);
    blocking(f).await.map_err(|e| throw_io(ctx, e, syscall, Some(path)))
}

async fn read_file<'js>(ctx: Ctx<'js>, path: String) -> Result<TypedArray<'js, u8>> {
    let p = path.clone();
    // std::fs::read sizes the buffer from fstat: one allocation.
    let data = path_call(&ctx, "open", &path, move || std::fs::read(p)).await?;
    TypedArray::new(ctx, data)
}

async fn write_file<'js>(ctx: Ctx<'js>, path: String, data: TypedArray<'js, u8>, options: Opt<Object<'js>>) -> Result<()> {
    let bytes = data.as_bytes().map(|b| b.to_vec()).unwrap_or_default();
    let (mode, append) = match options.0 {
        Some(o) => (o.get::<_, Option<u32>>("mode")?, o.get::<_, Option<bool>>("append")?.unwrap_or(false)),
        None => (None, false),
    };
    let p = path.clone();
    path_call(&ctx, "open", &path, move || {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true);
        if append { opts.append(true); } else { opts.truncate(true); }
        opts.mode(mode.unwrap_or(0o666));
        let mut f = opts.open(p)?;
        f.write_all(&bytes)
    })
    .await
}

fn stat_object<'js>(ctx: &Ctx<'js>, m: &std::fs::Metadata) -> Result<Object<'js>> {
    let ft = m.file_type();
    let kind = if ft.is_file() { "file" } else if ft.is_dir() { "dir" } else if ft.is_symlink() { "symlink" } else { "other" };
    let obj = Object::new(ctx.clone())?;
    obj.set("kind", kind)?;
    obj.set("mode", m.mode())?;
    obj.set("size", m.size() as f64)?;
    obj.set("mtimeMs", m.mtime() as f64 * 1000.0 + m.mtime_nsec() as f64 / 1e6)?;
    obj.set("atimeMs", m.atime() as f64 * 1000.0 + m.atime_nsec() as f64 / 1e6)?;
    obj.set("uid", m.uid())?;
    obj.set("gid", m.gid())?;
    obj.set("ino", m.ino() as f64)?;
    obj.set("dev", m.dev() as f64)?;
    obj.set("nlink", m.nlink() as f64)?;
    Ok(obj)
}

async fn stat<'js>(ctx: Ctx<'js>, path: String) -> Result<Object<'js>> {
    let p = path.clone();
    let m = path_call(&ctx, "stat", &path, move || std::fs::metadata(p)).await?;
    stat_object(&ctx, &m)
}

async fn lstat<'js>(ctx: Ctx<'js>, path: String) -> Result<Object<'js>> {
    let p = path.clone();
    let m = path_call(&ctx, "lstat", &path, move || std::fs::symlink_metadata(p)).await?;
    stat_object(&ctx, &m)
}

async fn readdir<'js>(ctx: Ctx<'js>, path: String) -> Result<Array<'js>> {
    let p = path.clone();
    let entries = path_call(&ctx, "scandir", &path, move || {
        let mut out: Vec<(String, &'static str)> = Vec::new();
        for entry in std::fs::read_dir(p)? {
            let entry = entry?;
            let ft = entry.file_type()?;
            let kind = if ft.is_file() { "file" } else if ft.is_dir() { "dir" } else if ft.is_symlink() { "symlink" } else { "other" };
            out.push((entry.file_name().to_string_lossy().into_owned(), kind));
        }
        Ok(out)
    })
    .await?;
    let arr = Array::new(ctx.clone())?;
    for (i, (name, kind)) in entries.into_iter().enumerate() {
        let obj = Object::new(ctx.clone())?;
        obj.set("name", name)?;
        obj.set("kind", kind)?;
        arr.set(i, obj)?;
    }
    Ok(arr)
}

async fn mkdir<'js>(ctx: Ctx<'js>, path: String, options: Opt<Object<'js>>) -> Result<()> {
    let (recursive, mode) = match options.0 {
        Some(o) => (o.get::<_, Option<bool>>("recursive")?.unwrap_or(false), o.get::<_, Option<u32>>("mode")?),
        None => (false, None),
    };
    let p = path.clone();
    path_call(&ctx, "mkdir", &path, move || {
        std::fs::DirBuilder::new().recursive(recursive).mode(mode.unwrap_or(0o777)).create(p)
    })
    .await
}

async fn rmdir<'js>(ctx: Ctx<'js>, path: String) -> Result<()> {
    let p = path.clone();
    path_call(&ctx, "rmdir", &path, move || std::fs::remove_dir(p)).await
}

async fn unlink<'js>(ctx: Ctx<'js>, path: String) -> Result<()> {
    let p = path.clone();
    path_call(&ctx, "unlink", &path, move || std::fs::remove_file(p)).await
}

async fn rename<'js>(ctx: Ctx<'js>, from: String, to: String) -> Result<()> {
    let (f, t) = (from.clone(), to.clone());
    path_call(&ctx, "rename", &from, move || std::fs::rename(f, t)).await
}

async fn symlink<'js>(ctx: Ctx<'js>, target: String, path: String) -> Result<()> {
    let (t, p) = (target.clone(), path.clone());
    path_call(&ctx, "symlink", &path, move || std::os::unix::fs::symlink(t, p)).await
}

async fn link<'js>(ctx: Ctx<'js>, from: String, to: String) -> Result<()> {
    let (f, t) = (from.clone(), to.clone());
    path_call(&ctx, "link", &from, move || std::fs::hard_link(f, t)).await
}

async fn readlink<'js>(ctx: Ctx<'js>, path: String) -> Result<String> {
    let p = path.clone();
    let target = path_call(&ctx, "readlink", &path, move || std::fs::read_link(p)).await?;
    Ok(target.to_string_lossy().into_owned())
}

async fn realpath<'js>(ctx: Ctx<'js>, path: String) -> Result<String> {
    let p = path.clone();
    let real = path_call(&ctx, "realpath", &path, move || std::fs::canonicalize(p)).await?;
    Ok(real.to_string_lossy().into_owned())
}

async fn chmod<'js>(ctx: Ctx<'js>, path: String, mode: u32) -> Result<()> {
    let p = path.clone();
    path_call(&ctx, "chmod", &path, move || {
        std::fs::set_permissions(p, std::os::unix::fs::PermissionsExt::from_mode(mode))
    })
    .await
}

async fn utimes<'js>(ctx: Ctx<'js>, path: String, atime_ms: f64, mtime_ms: f64) -> Result<()> {
    let p = path.clone();
    path_call(&ctx, "utime", &path, move || {
        let ts = |ms: f64| libc::timespec {
            tv_sec: (ms / 1000.0).floor() as libc::time_t,
            tv_nsec: ((ms % 1000.0 + 1000.0) % 1000.0 * 1e6) as _,
        };
        let times = [ts(atime_ms), ts(mtime_ms)];
        let cpath = std::ffi::CString::new(p).map_err(|_| io::Error::from_raw_os_error(libc::EINVAL))?;
        // SAFETY: valid C string and a two-element timespec array.
        let r = unsafe { libc::utimensat(libc::AT_FDCWD, cpath.as_ptr(), times.as_ptr(), 0) };
        if r != 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
    })
    .await
}

async fn truncate<'js>(ctx: Ctx<'js>, path: String, size: f64) -> Result<()> {
    let p = path.clone();
    path_call(&ctx, "open", &path, move || {
        std::fs::OpenOptions::new().write(true).open(p)?.set_len(size as u64)
    })
    .await
}

async fn open<'js>(ctx: Ctx<'js>, path: String, flags: String, mode: Opt<u32>) -> Result<i32> {
    let mut opts = std::fs::OpenOptions::new();
    let plus = flags.contains('+');
    match flags.trim_end_matches(['+', 'x', 's']) {
        "r" => { opts.read(true).write(plus); }
        "w" => { opts.write(true).create(true).truncate(true).read(plus); }
        "a" => { opts.append(true).create(true).read(plus); }
        _ => return Err(invalid(&ctx, "open", &format!("unknown flags '{flags}'"))),
    }
    if flags.contains('x') {
        opts.create_new(true);
    }
    opts.mode(mode.0.unwrap_or(0o666));
    let p = path.clone();
    let file = path_call(&ctx, "open", &path, move || opts.open(p)).await?;
    Ok(state(&ctx).handles.borrow_mut().insert(Resource::File(Arc::new(file))))
}

enum ReadTarget {
    File(Arc<std::fs::File>),
    Stream(Reader, std::rc::Rc<tokio::sync::Notify>),
}

fn take_reader<'js>(ctx: &Ctx<'js>, fd: i32) -> Result<ReadTarget> {
    let st = state(ctx);
    let mut handles = st.handles.borrow_mut();
    match handles.get_mut(fd) {
        None => Err(bad_handle(ctx, "read")),
        Some(Resource::File(f)) => Ok(ReadTarget::File(f.clone())),
        Some(Resource::Stream(s)) => match std::mem::replace(&mut s.reader, Slot::Busy) {
            Slot::Ready(r) => Ok(ReadTarget::Stream(r, s.cancel.clone())),
            Slot::Busy => Err(busy_handle(ctx, "read")),
            Slot::Absent => {
                s.reader = Slot::Absent;
                Err(bad_handle(ctx, "read"))
            }
        },
        Some(Resource::Listener(..)) | Some(Resource::Ws(_)) => Err(throw_errno(ctx, libc::ENOTSUP, "read", None)),
    }
}

fn restore_reader<'js>(ctx: &Ctx<'js>, fd: i32, reader: Reader) {
    if let Some(Resource::Stream(s)) = state(ctx).handles.borrow_mut().get_mut(fd) {
        s.reader = Slot::Ready(reader);
    }
}

/// Reads up to `max` bytes into a buffer allocated once at that size and
/// handed to JS without a copy. Resolves to `null` at end of stream.
pub async fn read<'js>(ctx: Ctx<'js>, fd: i32, max: u32) -> Result<Value<'js>> {
    let max = max as usize;
    if max == 0 {
        return Err(invalid(&ctx, "read", "max must be positive"));
    }
    let _activity = Activity::begin(&ctx);
    let result: io::Result<Vec<u8>> = match take_reader(&ctx, fd)? {
        ReadTarget::File(file) => {
            blocking(move || {
                let mut buf: Vec<u8> = Vec::with_capacity(max);
                let spare = buf.spare_capacity_mut();
                // SAFETY: read(2) writes into the uninitialised capacity and
                // reports how much of it is now initialised.
                let slice = unsafe { std::slice::from_raw_parts_mut(spare.as_mut_ptr() as *mut u8, spare.len()) };
                let n = (&*file).read(slice)?;
                unsafe { buf.set_len(n) };
                Ok(buf)
            })
            .await
        }
        ReadTarget::Stream(mut reader, cancel) => {
            let mut buf: Vec<u8> = Vec::with_capacity(max);
            let r = tokio::select! {
                r = reader.read_buf(&mut buf) => r.map(|_| ()),
                _ = cancel.notified() => Err(io::Error::from_raw_os_error(libc::ECANCELED)),
            };
            restore_reader(&ctx, fd, reader);
            r.map(|_| buf)
        }
    };
    let mut buf = result.map_err(|e| throw_io(&ctx, e, "read", None))?;
    if buf.is_empty() {
        return Ok(Value::new_null(ctx));
    }
    if buf.len() < buf.capacity() / 2 {
        buf.shrink_to_fit();
    }
    Ok(TypedArray::new(ctx, buf)?.into_value())
}

enum WriteTarget {
    File(Arc<std::fs::File>),
    Stream(Writer, std::rc::Rc<tokio::sync::Notify>),
}

fn take_writer<'js>(ctx: &Ctx<'js>, fd: i32) -> Result<WriteTarget> {
    let st = state(ctx);
    let mut handles = st.handles.borrow_mut();
    match handles.get_mut(fd) {
        None => Err(bad_handle(ctx, "write")),
        Some(Resource::File(f)) => Ok(WriteTarget::File(f.clone())),
        Some(Resource::Stream(s)) => match std::mem::replace(&mut s.writer, Slot::Busy) {
            Slot::Ready(w) => Ok(WriteTarget::Stream(w, s.cancel.clone())),
            Slot::Busy => Err(busy_handle(ctx, "write")),
            Slot::Absent => {
                s.writer = Slot::Absent;
                Err(bad_handle(ctx, "write"))
            }
        },
        Some(Resource::Listener(..)) | Some(Resource::Ws(_)) => Err(throw_errno(ctx, libc::ENOTSUP, "write", None)),
    }
}

fn restore_writer<'js>(ctx: &Ctx<'js>, fd: i32, writer: Writer) {
    if let Some(Resource::Stream(s)) = state(ctx).handles.borrow_mut().get_mut(fd) {
        s.writer = Slot::Ready(writer);
    }
}

/// Writes all of `data`; resolves once the bytes are in the kernel buffer,
/// which is the backpressure a caller sees.
pub async fn write<'js>(ctx: Ctx<'js>, fd: i32, data: TypedArray<'js, u8>) -> Result<()> {
    let _activity = Activity::begin(&ctx);
    let result: io::Result<()> = match take_writer(&ctx, fd)? {
        WriteTarget::File(file) => {
            let bytes = data.as_bytes().map(|b| b.to_vec()).unwrap_or_default();
            blocking(move || (&*file).write_all(&bytes)).await
        }
        WriteTarget::Stream(mut writer, cancel) => {
            let bytes = data.as_bytes().unwrap_or(&[]);
            let r = tokio::select! {
                r = writer.write_all(bytes) => r,
                _ = cancel.notified() => Err(io::Error::from_raw_os_error(libc::ECANCELED)),
            };
            restore_writer(&ctx, fd, writer);
            r
        }
    };
    result.map_err(|e| throw_io(&ctx, e, "write", None))
}

/// Releases a handle. A pending operation on a stream fails with `ECANCELED`.
pub fn close(ctx: Ctx<'_>, fd: i32) -> Result<()> {
    let removed = state(&ctx).handles.borrow_mut().remove(fd);
    match removed {
        None => Err(bad_handle(&ctx, "close")),
        Some(Resource::Stream(s)) => {
            s.cancel.notify_waiters();
            Ok(())
        }
        Some(Resource::Listener(_, cancel)) => {
            cancel.notify_waiters();
            Ok(())
        }
        Some(Resource::Ws(ws)) => {
            ws.cancel.notify_waiters();
            Ok(())
        }
        Some(Resource::File(_)) => Ok(()),
    }
}

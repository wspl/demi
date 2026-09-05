//! `ShellError`: the one error shape every primitive throws.
//!
//! Errors carry Node's string `code` (`ENOENT`, `EACCES`, ...), the raw
//! `errno`, the `syscall` and, when there is one, the `path`, so the Host
//! error mapping written for Node applies unchanged.

use std::io;

use rquickjs::{Ctx, Error, Exception, Result};

pub const ERRNO_CODES: &[(i32, &str)] = &[
    (libc::EPERM, "EPERM"),
    (libc::ENOENT, "ENOENT"),
    (libc::ESRCH, "ESRCH"),
    (libc::EINTR, "EINTR"),
    (libc::EIO, "EIO"),
    (libc::ENXIO, "ENXIO"),
    (libc::E2BIG, "E2BIG"),
    (libc::ENOEXEC, "ENOEXEC"),
    (libc::EBADF, "EBADF"),
    (libc::ECHILD, "ECHILD"),
    (libc::EAGAIN, "EAGAIN"),
    (libc::ENOMEM, "ENOMEM"),
    (libc::EACCES, "EACCES"),
    (libc::EFAULT, "EFAULT"),
    (libc::EBUSY, "EBUSY"),
    (libc::EEXIST, "EEXIST"),
    (libc::EXDEV, "EXDEV"),
    (libc::ENODEV, "ENODEV"),
    (libc::ENOTDIR, "ENOTDIR"),
    (libc::EISDIR, "EISDIR"),
    (libc::EINVAL, "EINVAL"),
    (libc::ENFILE, "ENFILE"),
    (libc::EMFILE, "EMFILE"),
    (libc::ENOTTY, "ENOTTY"),
    (libc::ETXTBSY, "ETXTBSY"),
    (libc::EFBIG, "EFBIG"),
    (libc::ENOSPC, "ENOSPC"),
    (libc::ESPIPE, "ESPIPE"),
    (libc::EROFS, "EROFS"),
    (libc::EMLINK, "EMLINK"),
    (libc::EPIPE, "EPIPE"),
    (libc::ERANGE, "ERANGE"),
    (libc::EDEADLK, "EDEADLK"),
    (libc::ENAMETOOLONG, "ENAMETOOLONG"),
    (libc::ENOSYS, "ENOSYS"),
    (libc::ENOTEMPTY, "ENOTEMPTY"),
    (libc::ELOOP, "ELOOP"),
    (libc::ENOMSG, "ENOMSG"),
    (libc::EOVERFLOW, "EOVERFLOW"),
    (libc::EILSEQ, "EILSEQ"),
    (libc::ENOTSOCK, "ENOTSOCK"),
    (libc::EDESTADDRREQ, "EDESTADDRREQ"),
    (libc::EMSGSIZE, "EMSGSIZE"),
    (libc::EPROTOTYPE, "EPROTOTYPE"),
    (libc::ENOPROTOOPT, "ENOPROTOOPT"),
    (libc::EPROTONOSUPPORT, "EPROTONOSUPPORT"),
    (libc::ENOTSUP, "ENOTSUP"),
    (libc::EAFNOSUPPORT, "EAFNOSUPPORT"),
    (libc::EADDRINUSE, "EADDRINUSE"),
    (libc::EADDRNOTAVAIL, "EADDRNOTAVAIL"),
    (libc::ENETDOWN, "ENETDOWN"),
    (libc::ENETUNREACH, "ENETUNREACH"),
    (libc::ENETRESET, "ENETRESET"),
    (libc::ECONNABORTED, "ECONNABORTED"),
    (libc::ECONNRESET, "ECONNRESET"),
    (libc::ENOBUFS, "ENOBUFS"),
    (libc::EISCONN, "EISCONN"),
    (libc::ENOTCONN, "ENOTCONN"),
    (libc::ETIMEDOUT, "ETIMEDOUT"),
    (libc::ECONNREFUSED, "ECONNREFUSED"),
    (libc::EHOSTUNREACH, "EHOSTUNREACH"),
    (libc::EALREADY, "EALREADY"),
    (libc::EINPROGRESS, "EINPROGRESS"),
    (libc::ESTALE, "ESTALE"),
    (libc::EDQUOT, "EDQUOT"),
    (libc::ECANCELED, "ECANCELED"),
    (libc::EPROTO, "EPROTO"),
];

pub fn errno_code(errno: i32) -> &'static str {
    ERRNO_CODES
        .iter()
        .find(|(n, _)| *n == errno)
        .map(|(_, c)| *c)
        .unwrap_or("EUNKNOWN")
}

fn strerror(errno: i32) -> String {
    // SAFETY: strerror returns a pointer to a static, NUL-terminated string.
    let text = unsafe { std::ffi::CStr::from_ptr(libc::strerror(errno)) };
    let text = text.to_string_lossy();
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Builds and throws a `ShellError` for an OS error.
pub fn throw_io(ctx: &Ctx<'_>, err: io::Error, syscall: &str, path: Option<&str>) -> Error {
    match err.raw_os_error() {
        Some(errno) => throw_errno(ctx, errno, syscall, path),
        // Errors we synthesised ourselves carry their text and no errno.
        None => throw_code(ctx, other_code(&err), &err.to_string(), syscall, path),
    }
}

/// The code for an `io::Error` that has no errno.
fn other_code(err: &io::Error) -> &'static str {
    use io::ErrorKind::*;
    let text = err.to_string();
    if text.contains("dns error") || text.contains("failed to lookup") {
        return "ENOTFOUND";
    }
    match err.kind() {
        UnexpectedEof | BrokenPipe => "EPIPE",
        InvalidInput | InvalidData => "EINVAL",
        TimedOut => "ETIMEDOUT",
        NotFound => "ENOENT",
        PermissionDenied => "EACCES",
        _ => "EPROTO",
    }
}

pub fn throw_errno(ctx: &Ctx<'_>, errno: i32, syscall: &str, path: Option<&str>) -> Error {
    let code = errno_code(errno);
    let message = format!("{code}: {}", strerror(errno));
    make_error(ctx, code, errno, &message, syscall, path)
}

/// Throws a `ShellError` that has a code but no errno (protocol errors,
/// argument errors).
pub fn throw_code(ctx: &Ctx<'_>, code: &str, detail: &str, syscall: &str, path: Option<&str>) -> Error {
    let message = format!("{code}: {detail}");
    make_error(ctx, code, 0, &message, syscall, path)
}

fn make_error(ctx: &Ctx<'_>, code: &str, errno: i32, message: &str, syscall: &str, path: Option<&str>) -> Error {
    let message = match path {
        Some(p) => format!("{message}, {syscall} '{p}'"),
        None => format!("{message}, {syscall}"),
    };
    let build = || -> Result<Error> {
        let obj = Exception::from_message(ctx.clone(), &message)?.into_object();
        obj.set("name", "ShellError")?;
        obj.set("code", code)?;
        obj.set("errno", errno)?;
        obj.set("syscall", syscall)?;
        if let Some(p) = path {
            obj.set("path", p)?;
        }
        Ok(ctx.throw(obj.into_value()))
    };
    build().unwrap_or_else(|e| e)
}

/// `EBADF` for a handle that is not in the table.
pub fn bad_handle(ctx: &Ctx<'_>, syscall: &str) -> Error {
    throw_errno(ctx, libc::EBADF, syscall, None)
}

/// `EBUSY` for a second operation of the same direction on a handle.
pub fn busy_handle(ctx: &Ctx<'_>, syscall: &str) -> Error {
    throw_errno(ctx, libc::EBUSY, syscall, None)
}

/// `EINVAL` with a description, for argument errors.
pub fn invalid(ctx: &Ctx<'_>, syscall: &str, detail: &str) -> Error {
    throw_code(ctx, "EINVAL", detail, syscall, None)
}

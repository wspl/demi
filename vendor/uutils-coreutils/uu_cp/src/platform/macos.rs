// This file is part of the uutils coreutils package.
//
// For the full copyright and license information, please view the LICENSE
// file that was distributed with this source code.
// spell-checker:ignore reflink
use std::ffi::CString;
use uucore::context::fs::{File, OpenOptions};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use uucore::buf_copy;
use uucore::display::Quotable;
use uucore::safe_copy::{create_dest_restrictive, open_source};
use uucore::translate;

use uucore::mode::get_umask;

use crate::{
    CopyDebug, CopyResult, CpError, OffloadReflinkDebug, ReflinkMode, SparseDebug, SparseMode,
    is_stream,
};

/// Copies `source` to `dest` using copy-on-write if possible.
pub(crate) fn copy_on_write(
    source: &Path,
    dest: &Path,
    reflink_mode: ReflinkMode,
    sparse_mode: SparseMode,
    context: &str,
    source_is_stream: bool,
    nofollow: bool,
) -> CopyResult<CopyDebug> {
    if sparse_mode != SparseMode::Auto {
        return Err(translate!("cp-error-sparse-not-supported").into());
    }
    let mut copy_debug = CopyDebug {
        offload: OffloadReflinkDebug::Unknown,
        reflink: OffloadReflinkDebug::Unsupported,
        sparse_detection: SparseDebug::Unsupported,
    };

    // Extract paths in a form suitable to be passed to a syscall.
    // The unwrap() is safe because they come from the command-line and so contain non nul
    // character.
    let src = CString::new(uucore::context::resolve(source).as_os_str().as_bytes()).unwrap();
    let dst = CString::new(uucore::context::resolve(dest).as_os_str().as_bytes()).unwrap();

    // clonefile(2) was introduced in macOS 10.12 so we cannot statically link against it
    // for backward compatibility.
    let clonefile = CString::new("clonefile").unwrap();
    let raw_pfn = unsafe { libc::dlsym(libc::RTLD_NEXT, clonefile.as_ptr()) };

    let mut error = 0;
    // `--reflink=never` must not clone: clonefile(2) COW-shares the source's blocks and copies
    // its metadata (including mtime), so skip it here and fall through to a normal byte copy
    // below, matching GNU/BSD `cp` (and the Linux path's ReflinkMode::Never handling).
    let attempt_clone = !raw_pfn.is_null() && reflink_mode != ReflinkMode::Never;
    if attempt_clone {
        // Call clonefile(2).
        // Safety: Casting a C function pointer to a rust function value is one of the few
        // blessed uses of `transmute()`.
        unsafe {
            let pfn: extern "C" fn(
                src: *const core::ffi::c_char,
                dst: *const core::ffi::c_char,
                flags: u32,
            ) -> core::ffi::c_int = std::mem::transmute(raw_pfn);
            // Demi's: clonefile(2) fails when the destination exists, and
            // the copy below then writes it in place, as GNU cp does, so a
            // hard link to it stays one; removing it to clone again would
            // make a new file.
            error = pfn(src.as_ptr(), dst.as_ptr(), 0);
        }
        if error == 0 {
            // clonefile(2) copied the source's times and extended
            // attributes too; a copy has the time it was made and no
            // attributes, as GNU cp's has, unless it is to preserve them,
            // which the caller does after this.
            let now = filetime::FileTime::now();
            filetime::set_file_times(uucore::context::resolve(dest), now, now)
                .map_err(|e| CpError::IoErrContext(e, context.to_owned()))?;
            uucore::fsxattr::remove_xattrs(dest)
                .map_err(|e| CpError::IoErrContext(e, context.to_owned()))?;
        }
    }

    if !attempt_clone || error != 0 {
        // clonefile(2) is either not supported or it errored out (possibly because the FS does not
        // support COW).
        if reflink_mode == ReflinkMode::Always {
            return Err(translate!("cp-error-failed-to-clone", "source" => source.quote(), "dest" => dest.quote(), "error" => error)
                .into());
        }
        copy_debug.reflink = OffloadReflinkDebug::Yes;
        if source_is_stream {
            let mut src_file =
                File::open(source).map_err(|e| CpError::IoErrContext(e, context.to_owned()))?;
            let mode = 0o622 & !get_umask();
            let mut dst_file = OpenOptions::new()
                .create(true)
                .write(true)
                .mode(mode)
                .open(dest)
                .map_err(|e| {
                    CpError::IoErrContext(
                        e,
                        translate!("cp-error-cannot-create-regular-file", "path" => dest.quote()),
                    )
                })?;

            let dest_is_stream = is_stream(
                &dst_file
                    .metadata()
                    .map_err(|e| CpError::IoErrContext(e, context.to_owned()))?,
            );
            if !dest_is_stream {
                // `copy_stream` doesn't clear the dest file, if dest is not a stream, we should clear it manually.
                dst_file
                    .set_len(0)
                    .map_err(|e| CpError::IoErrContext(e, context.to_owned()))?;
            }

            buf_copy::copy_fast(&mut src_file, &mut dst_file)
                .map_err(|_| uucore::context::io::Error::from(uucore::context::io::ErrorKind::Other))
                .map_err(|e| CpError::IoErrContext(e, context.to_owned()))?;
        } else {
            let mut src_file = open_source(source, nofollow)
                .map_err(|e| CpError::IoErrContext(e, context.to_owned()))?;
            let mut dst_file = create_dest_restrictive(dest, false).map_err(|e| {
                CpError::IoErrContext(
                    e,
                    translate!("cp-error-cannot-create-regular-file", "path" => dest.quote()),
                )
            })?;
            uucore::context::io::copy(&mut src_file, &mut dst_file)
                .map_err(|e| CpError::IoErrContext(e, context.to_owned()))?;
        }
    }

    Ok(copy_debug)
}

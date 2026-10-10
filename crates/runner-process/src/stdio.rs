//! Standard handles, and the identity of a job's own input (`runner.md`
//! § A job's own input).

use std::{collections::BTreeMap, fs::File, io};

/// Names the job's stdin pipe, when it is the job's own input; absent for a
/// job whose stdin the backend relays from another command.
pub const LIVE_INPUT_ENV: &str = "DEMI_LIVE_INPUT";

pub fn standard_file(descriptor: u32) -> io::Result<File> {
    #[cfg(unix)]
    {
        use std::os::fd::AsFd;
        let handle = match descriptor {
            0 => std::io::stdin().as_fd().try_clone_to_owned()?,
            1 => std::io::stdout().as_fd().try_clone_to_owned()?,
            _ => std::io::stderr().as_fd().try_clone_to_owned()?,
        };
        Ok(handle.into())
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsHandle;
        let handle = match descriptor {
            0 => std::io::stdin().as_handle().try_clone_to_owned()?,
            1 => std::io::stdout().as_handle().try_clone_to_owned()?,
            _ => std::io::stderr().as_handle().try_clone_to_owned()?,
        };
        Ok(handle.into())
    }
}

/// The reference a job's environment names `file` by. The caller keeps
/// `file` open for the entire shell job.
pub fn reference(file: &File) -> io::Result<String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = file.metadata()?;
        Ok(format!("{}:{}", metadata.dev(), metadata.ino()))
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        Ok(format!(
            "{}:{}",
            std::process::id(),
            file.as_raw_handle() as usize
        ))
    }
}

/// Whether `file`, a process's stdin, is the job's own input in any copy.
pub fn is_live(file: &File, env: &BTreeMap<String, String>) -> io::Result<bool> {
    let name = LIVE_INPUT_ENV;
    let Some(reference) = env.get(name) else {
        return Ok(false);
    };
    #[cfg(unix)]
    {
        Ok(self::reference(file)? == *reference)
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
        use windows_sys::Win32::{
            Foundation::{CompareObjectHandles, DUPLICATE_SAME_ACCESS, DuplicateHandle},
            System::Threading::{GetCurrentProcess, OpenProcess, PROCESS_DUP_HANDLE},
        };
        let (pid, handle) = reference
            .split_once(':')
            .ok_or_else(|| io::Error::other(format!("invalid reference in {name}")))?;
        let pid: u32 = pid.parse().map_err(io::Error::other)?;
        let handle: usize = handle.parse().map_err(io::Error::other)?;
        let process = unsafe { OpenProcess(PROCESS_DUP_HANDLE, 0, pid) };
        if process.is_null() {
            return Err(io::Error::last_os_error());
        }
        let process = unsafe { OwnedHandle::from_raw_handle(process) };
        let mut duplicated = std::ptr::null_mut();
        if unsafe {
            DuplicateHandle(
                process.as_raw_handle(),
                handle as _,
                GetCurrentProcess(),
                &mut duplicated,
                0,
                0,
                DUPLICATE_SAME_ACCESS,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        let duplicated = unsafe { OwnedHandle::from_raw_handle(duplicated) };
        Ok(unsafe { CompareObjectHandles(file.as_raw_handle(), duplicated.as_raw_handle()) } != 0)
    }
}

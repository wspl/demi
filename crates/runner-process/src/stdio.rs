//! A process's standard handles as files of its own, and whether one is the
//! null device, which a job's stdin is when nothing is relayed to it
//! (`crates/runner-shell/src/job.rs`).

use std::{fs::File, io};

/// A copy of the process's standard input (0), output (1) or error (any
/// other).
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

/// Whether `file` is the null device: a command whose stdin it is has no
/// input, so it passes none on, as `demi host shell` would to its far job.
pub fn is_null_device(file: &File) -> io::Result<bool> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{FileTypeExt, MetadataExt};
        let metadata = file.metadata()?;
        if !metadata.file_type().is_char_device() {
            return Ok(false);
        }
        Ok(metadata.rdev() == std::fs::metadata("/dev/null")?.rdev())
    }
    #[cfg(windows)]
    {
        // A job's stdin is a pipe, a file or `NUL`, the one character
        // device among them.
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{FILE_TYPE_CHAR, GetFileType};
        Ok(unsafe { GetFileType(file.as_raw_handle()) } == FILE_TYPE_CHAR)
    }
}

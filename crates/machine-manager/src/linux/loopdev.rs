//! Loop devices the manager attaches itself: each working image gets a free
//! device with auto-clear set, so unmounting its filesystem, or a mount
//! that fails, detaches the device.
//!
//! `LOOP_CONFIGURE` attaches and configures a device in one call from Linux
//! 5.8. An older kernel refuses it, and the manager attaches with
//! `LOOP_SET_FD` and then sets auto-clear with `LOOP_SET_STATUS64`, as
//! util-linux's `losetup` does; a device whose status cannot be set is
//! detached again.

use std::{
    io,
    os::fd::{AsFd, AsRawFd},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

use linux_raw_sys::loop_device::{
    LO_FLAGS_AUTOCLEAR, LO_FLAGS_READ_ONLY, LOOP_CLR_FD, LOOP_CONFIGURE, LOOP_CTL_GET_FREE, LOOP_SET_DIRECT_IO,
    LOOP_SET_FD, LOOP_SET_STATUS64, loop_config, loop_info64,
};
use rustix::{
    io::Errno,
    ioctl::{IntegerSetter, Ioctl, IoctlOutput, NoArg, Opcode, Setter},
};

/// Whether the kernel refused `LOOP_CONFIGURE` once, so every later attach
/// goes the older way at once.
static NO_CONFIGURE: AtomicBool = AtomicBool::new(false);

use super::failed;
use crate::blocking::OffLoop;

/// An attached loop device, held open until its filesystem is mounted.
/// Dropping it before a mount holds the device detaches it.
pub struct LoopDevice {
    number: u32,
    _device: fs_err::File,
}

impl LoopDevice {
    pub fn path(&self) -> PathBuf {
        path(self.number)
    }
}

/// `/dev/loop<number>`.
pub fn path(number: u32) -> PathBuf {
    PathBuf::from(format!("/dev/loop{number}"))
}

/// `LOOP_CTL_GET_FREE`: the number of a free loop device, which the kernel
/// allocates when none is free. The number is the call's return value.
struct GetFree;

// SAFETY: LOOP_CTL_GET_FREE takes no argument and reads or writes no memory;
// its result is the return value.
unsafe impl Ioctl for GetFree {
    type Output = u32;

    const IS_MUTATING: bool = false;

    fn opcode(&self) -> Opcode {
        LOOP_CTL_GET_FREE
    }

    fn as_ptr(&mut self) -> *mut std::ffi::c_void {
        std::ptr::null_mut()
    }

    unsafe fn output_from_ptr(
        out: IoctlOutput,
        _: *mut std::ffi::c_void,
    ) -> rustix::io::Result<u32> {
        u32::try_from(out).map_err(|_| Errno::RANGE)
    }
}

/// Has the attached loop device `device` read and write its backing file
/// with direct I/O, so the file's pages are not cached a second time beside
/// the filesystem's on it.
pub fn set_direct_io(_: &OffLoop, device: &Path) -> io::Result<()> {
    let opened = fs_err::OpenOptions::new().read(true).open(device)?;
    // SAFETY: LOOP_SET_DIRECT_IO takes its argument by value, 1 to turn
    // direct I/O on.
    unsafe { rustix::ioctl::ioctl(opened.as_fd(), IntegerSetter::<{ LOOP_SET_DIRECT_IO }>::new_usize(1)) }
        .map_err(|error| failed("turning direct I/O on for", device, error))
}

/// Attaches `image` to a free loop device with auto-clear set.
pub fn attach(off: &OffLoop, image: &Path) -> io::Result<LoopDevice> {
    attach_as(off, image, false)
}

/// Attaches `image` to a free loop device that refuses writes, with
/// auto-clear set: a committed image, which nothing may change.
pub fn attach_read_only(off: &OffLoop, image: &Path) -> io::Result<LoopDevice> {
    attach_as(off, image, true)
}

fn attach_as(_: &OffLoop, image: &Path, read_only: bool) -> io::Result<LoopDevice> {
    let backing = fs_err::OpenOptions::new()
        .read(true)
        .write(!read_only)
        .open(image)?;
    let control = fs_err::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/loop-control")?;
    loop {
        // SAFETY: GetFree matches LOOP_CTL_GET_FREE, which /dev/loop-control serves.
        let number =
            unsafe { rustix::ioctl::ioctl(control.as_fd(), GetFree) }.map_err(|error| {
                failed(
                    "finding a free loop device through",
                    Path::new("/dev/loop-control"),
                    error,
                )
            })?;
        let device_path = path(number);
        let device = fs_err::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&device_path)?;
        let flags = match read_only {
            true => LO_FLAGS_AUTOCLEAR as u32 | LO_FLAGS_READ_ONLY as u32,
            false => LO_FLAGS_AUTOCLEAR as u32,
        };
        let configured = match NO_CONFIGURE.load(Ordering::Relaxed) {
            false => match configure(&device, &backing, flags) {
                // A kernel before 5.8 knows no LOOP_CONFIGURE, and refuses
                // it as a newer one refuses a backing it cannot take; only
                // the older way's success says which it was.
                Err(Errno::INVAL | Errno::NOTTY) => {
                    let attached = set_fd(&device, &backing, flags);
                    if attached.is_ok() {
                        NO_CONFIGURE.store(true, Ordering::Relaxed);
                    }
                    attached
                }
                other => other,
            },
            true => set_fd(&device, &backing, flags),
        };
        match configured {
            Ok(()) => {
                return Ok(LoopDevice {
                    number,
                    _device: device,
                });
            }
            // Another process took the device between the two calls.
            Err(Errno::BUSY) => continue,
            Err(error) => return Err(failed("attaching", &device_path, error)),
        }
    }
}

/// Attaches `backing` to `device` with `flags` set, in one call.
fn configure(device: &fs_err::File, backing: &fs_err::File, flags: u32) -> rustix::io::Result<()> {
    // SAFETY: an all-zero loop_config is valid: plain integers and byte
    // arrays; the fields that matter are set below.
    let mut config: loop_config = unsafe { std::mem::zeroed() };
    config.fd = u32::try_from(backing.as_raw_fd()).expect("a file descriptor is not negative");
    config.info.lo_flags = flags;
    // SAFETY: LOOP_CONFIGURE reads a loop_config from the pointer the Setter
    // passes, and the backing descriptor stays open for the call.
    unsafe { rustix::ioctl::ioctl(device.as_fd(), Setter::<{ LOOP_CONFIGURE }, loop_config>::new(config)) }
}

/// Attaches `backing` to `device`, then sets `flags`: the way before
/// `LOOP_CONFIGURE`. A device whose status cannot be set is detached.
fn set_fd(device: &fs_err::File, backing: &fs_err::File, flags: u32) -> rustix::io::Result<()> {
    let fd = usize::try_from(backing.as_raw_fd()).expect("a file descriptor is not negative");
    // SAFETY: LOOP_SET_FD takes the backing descriptor by value, which stays
    // open for the call.
    unsafe { rustix::ioctl::ioctl(device.as_fd(), IntegerSetter::<{ LOOP_SET_FD }>::new_usize(fd)) }?;
    // SAFETY: an all-zero loop_info64 is valid: plain integers and byte
    // arrays; only the flags are set.
    let mut info: loop_info64 = unsafe { std::mem::zeroed() };
    info.lo_flags = flags;
    // SAFETY: LOOP_SET_STATUS64 reads a loop_info64 from the pointer the
    // Setter passes.
    let status = unsafe { rustix::ioctl::ioctl(device.as_fd(), Setter::<{ LOOP_SET_STATUS64 }, loop_info64>::new(info)) };
    if status.is_err() {
        // SAFETY: LOOP_CLR_FD takes no argument; the device is the one just
        // attached.
        let _ = unsafe { rustix::ioctl::ioctl(device.as_fd(), NoArg::<{ LOOP_CLR_FD }>::new()) };
    }
    status
}

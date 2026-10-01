//! Loop devices the manager attaches itself (`managed-hosts.md` § Images):
//! each working image gets a free device with auto-clear set, so unmounting
//! its filesystem, or a mount that fails, detaches the device. The sandbox
//! keeps the device's number for growth.

use std::{
    io,
    os::fd::{AsFd, AsRawFd},
    path::{Path, PathBuf},
};

use linux_raw_sys::loop_device::{
    LO_FLAGS_AUTOCLEAR, LOOP_CONFIGURE, LOOP_CTL_GET_FREE, LOOP_SET_CAPACITY, loop_config,
};
use rustix::{
    io::Errno,
    ioctl::{Ioctl, IoctlOutput, NoArg, Opcode, Setter},
};

use super::failed;
use crate::blocking::OffLoop;

/// An attached loop device, held open until its filesystem is mounted.
/// Dropping it before a mount holds the device detaches it.
pub struct LoopDevice {
    number: u32,
    _device: fs_err::File,
}

impl LoopDevice {
    pub fn number(&self) -> u32 {
        self.number
    }

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

    unsafe fn output_from_ptr(out: IoctlOutput, _: *mut std::ffi::c_void) -> rustix::io::Result<u32> {
        u32::try_from(out).map_err(|_| Errno::RANGE)
    }
}

/// Attaches `image` to a free loop device with auto-clear set.
pub fn attach(_: &OffLoop, image: &Path) -> io::Result<LoopDevice> {
    let backing = fs_err::OpenOptions::new().read(true).write(true).open(image)?;
    let control = fs_err::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/loop-control")?;
    loop {
        // SAFETY: GetFree matches LOOP_CTL_GET_FREE, which /dev/loop-control serves.
        let number = unsafe { rustix::ioctl::ioctl(control.as_fd(), GetFree) }
            .map_err(|error| failed("finding a free loop device through", Path::new("/dev/loop-control"), error))?;
        let device_path = path(number);
        let device = fs_err::OpenOptions::new().read(true).write(true).open(&device_path)?;
        // SAFETY: an all-zero loop_config is valid: plain integers and byte
        // arrays; the fields that matter are set below.
        let mut config: loop_config = unsafe { std::mem::zeroed() };
        config.fd = u32::try_from(backing.as_raw_fd()).expect("a file descriptor is not negative");
        config.info.lo_flags = LO_FLAGS_AUTOCLEAR as u32;
        // SAFETY: LOOP_CONFIGURE reads a loop_config from the pointer the
        // Setter passes, and the backing descriptor stays open for the call.
        let configured = unsafe {
            rustix::ioctl::ioctl(device.as_fd(), Setter::<{ LOOP_CONFIGURE }, loop_config>::new(config))
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

/// Makes loop device `number` see its backing file's new size.
pub fn refresh_capacity(_: &OffLoop, number: u32) -> io::Result<()> {
    let device_path = path(number);
    let device = fs_err::OpenOptions::new().read(true).write(true).open(&device_path)?;
    // SAFETY: LOOP_SET_CAPACITY takes no argument and is served by a loop device.
    unsafe { rustix::ioctl::ioctl(device.as_fd(), NoArg::<{ LOOP_SET_CAPACITY }>::new()) }
        .map_err(|error| failed("refreshing the capacity of", &device_path, error))
}

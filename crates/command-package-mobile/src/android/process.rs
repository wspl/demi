//! An emulator's process, which need not be this program's child: an
//! emulator Demi started outlives the program (`mobile.md` § Lifetime).
//! Its id and start tell it from a later process the system gives the same
//! id; its end is what the system announces, not a guess from its files.

use std::time::Duration;

/// A process, by its id and the second it started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Process {
    pub pid: u32,
    pub started: u64,
}

/// How long a process asked to end may take before it is killed.
const ENDING: Duration = Duration::from_secs(30);

impl Process {
    /// The process `pid` names now, or none.
    pub fn of(pid: u32) -> Option<Self> {
        let id = sysinfo::Pid::from_u32(pid);
        let mut system = sysinfo::System::new();
        system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[id]), true);
        system.process(id).map(|process| Self {
            pid,
            started: process.start_time(),
        })
    }

    /// Whether this process still runs, rather than another with its id.
    pub fn alive(&self) -> bool {
        Self::of(self.pid) == Some(*self)
    }

    /// Ends once the process has ended, as the system announces it.
    pub async fn ended(&self) {
        let watched = exit::Exit::watch(self.pid);
        // Watched first, then checked: an end between the two is announced.
        if !self.alive() {
            return;
        }
        match watched {
            Ok(exit) => exit.ended().await,
            // The system no longer has the process to watch.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                tracing::warn!("the end of process {} cannot be watched: {error}", self.pid);
                std::future::pending::<()>().await;
            }
        }
    }

    /// Asks the process to end and waits until it has, killing it when it
    /// takes longer than [`ENDING`]; a process that is not this one any
    /// more is left alone. Answers why it did not end.
    pub async fn end(&self) -> Result<(), String> {
        if !self.signal(sysinfo::Signal::Term) {
            return Ok(());
        }
        if tokio::time::timeout(ENDING, self.ended()).await.is_ok() {
            return Ok(());
        }
        self.signal(sysinfo::Signal::Kill);
        tokio::time::timeout(ENDING, self.ended())
            .await
            .map_err(|_| format!("process {} did not end", self.pid))
    }

    /// Sends `signal` when this process still runs; answers whether it did.
    fn signal(&self, signal: sysinfo::Signal) -> bool {
        let id = sysinfo::Pid::from_u32(self.pid);
        let mut system = sysinfo::System::new();
        system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[id]), true);
        match system.process(id) {
            Some(process) if process.start_time() == self.started => process.kill_with(signal).unwrap_or(false),
            _ => false,
        }
    }
}

#[cfg(target_os = "macos")]
mod exit {
    //! A process's exit through kqueue's `EVFILT_PROC`, whose queue tokio
    //! waits on as on any descriptor.

    use kqueue::{EventFilter, FilterFlag, Watcher};
    use tokio::io::unix::AsyncFd;

    pub struct Exit(AsyncFd<Watcher>);

    impl Exit {
        pub fn watch(pid: u32) -> std::io::Result<Self> {
            let mut watcher = Watcher::new()?;
            let pid = i32::try_from(pid).map_err(|_| std::io::Error::from(std::io::ErrorKind::NotFound))?;
            watcher.add_pid(pid, EventFilter::EVFILT_PROC, FilterFlag::NOTE_EXIT)?;
            // A process that ended already cannot be added: ESRCH.
            watcher.watch().map_err(|error| match error.raw_os_error() {
                Some(3) => std::io::Error::from(std::io::ErrorKind::NotFound),
                _ => error,
            })?;
            // Only readable: macOS refuses a kqueue watched for writing.
            Ok(Self(AsyncFd::with_interest(watcher, tokio::io::Interest::READABLE)?))
        }

        pub async fn ended(self) {
            loop {
                let Ok(mut ready) = self.0.readable().await else { return };
                if self.0.get_ref().poll(None).is_some() {
                    return;
                }
                ready.clear_ready();
            }
        }
    }
}

#[cfg(target_os = "linux")]
mod exit {
    //! A process's exit through a pidfd, readable once the process ended.

    use std::os::fd::OwnedFd;

    use tokio::io::unix::AsyncFd;

    pub struct Exit(AsyncFd<OwnedFd>);

    impl Exit {
        pub fn watch(pid: u32) -> std::io::Result<Self> {
            let pid = i32::try_from(pid)
                .ok()
                .and_then(rustix::process::Pid::from_raw)
                .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))?;
            let fd = rustix::process::pidfd_open(pid, rustix::process::PidfdFlags::NONBLOCK).map_err(|error| {
                if error == rustix::io::Errno::SRCH {
                    std::io::Error::from(std::io::ErrorKind::NotFound)
                } else {
                    std::io::Error::from(error)
                }
            })?;
            Ok(Self(AsyncFd::new(fd)?))
        }

        pub async fn ended(self) {
            // A pidfd turns readable once, as its process ends.
            let _ready = self.0.readable().await;
        }
    }
}

#[cfg(windows)]
mod exit {
    //! A process's exit through its handle, which a thread of the blocking
    //! pool waits on, waking each second to see whether the wait is still
    //! wanted.

    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE, WAIT_TIMEOUT},
        System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
    };

    pub struct Exit(Handle);

    /// The process's handle, closed as it is dropped.
    struct Handle(HANDLE);

    // SAFETY: a process handle is a kernel object's handle, usable from
    // any thread.
    unsafe impl Send for Handle {}

    impl Drop for Handle {
        fn drop(&mut self) {
            // SAFETY: the handle is this value's own, open until now.
            unsafe { CloseHandle(self.0) };
        }
    }

    impl Exit {
        pub fn watch(pid: u32) -> std::io::Result<Self> {
            // SAFETY: OpenProcess takes plain values and answers a handle or null.
            let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
            if handle.is_null() {
                return Err(std::io::Error::from(std::io::ErrorKind::NotFound));
            }
            Ok(Self(Handle(handle)))
        }

        pub async fn ended(self) {
            /// Stops the waiting thread once nobody waits for it.
            struct Wanted(Arc<AtomicBool>);
            impl Drop for Wanted {
                fn drop(&mut self) {
                    self.0.store(false, Ordering::Relaxed);
                }
            }
            let wanted = Wanted(Arc::new(AtomicBool::new(true)));
            let still = wanted.0.clone();
            let handle = self.0;
            let _waited = tokio::task::spawn_blocking(move || {
                while still.load(Ordering::Relaxed) {
                    // SAFETY: the handle stays open while this closure owns it.
                    if unsafe { WaitForSingleObject(handle.0, 1000) } != WAIT_TIMEOUT {
                        return;
                    }
                }
            })
            .await;
        }
    }
}

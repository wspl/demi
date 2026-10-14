//! A pseudo-terminal for the scenarios that run a program the way a person
//! at a terminal does: as the leader of a session that the terminal
//! controls, in its foreground, so that the terminal's Ctrl-C reaches the
//! program's process group, and a shell started there with `-i` has job
//! control.

use std::{
    io::{self, Write as _},
    os::{fd::BorrowedFd, fd::OwnedFd, unix::net::UnixStream},
};

use rustix::{
    fs::{Mode, OFlags},
    pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt},
};

/// A pseudo-terminal, which the test holds as a person's terminal window
/// would.
pub struct Terminal {
    /// The side the person's window holds: what it reads is what the
    /// terminal shows, and what it writes is typed.
    main: OwnedFd,
}

impl Terminal {
    /// A new terminal; an error where the system gives none, as a
    /// container without `/dev/ptmx` does.
    pub fn open() -> io::Result<Self> {
        let main = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY)?;
        // No program the test starts holds the window's side: the terminal
        // would stay open with it. macOS opens no terminal close-on-exec.
        rustix::io::fcntl_setfd(&main, rustix::io::FdFlags::CLOEXEC)?;
        grantpt(&main)?;
        unlockpt(&main)?;
        Ok(Self { main })
    }

    /// Starts `command` as the leader of a new session that this terminal
    /// controls, its process group the terminal's foreground one, with the
    /// terminal as its standard input, output and error.
    pub fn spawn(&self, mut command: tokio::process::Command) -> io::Result<tokio::process::Child> {
        let name = ptsname(&self.main, Vec::new())?;
        let terminal: OwnedFd = rustix::fs::open(
            name.as_c_str(),
            OFlags::RDWR | OFlags::NOCTTY | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        command
            .stdin(terminal.try_clone()?)
            .stdout(terminal.try_clone()?)
            .stderr(terminal);
        // SAFETY: the hook runs in the child between fork and exec, once
        // its standard files are the terminal, and makes only `setsid` and
        // `ioctl` calls, which are async-signal-safe, and allocates
        // nothing. Descriptor 0 is open for the hook's whole run.
        unsafe {
            command.pre_exec(|| {
                rustix::process::setsid()?;
                rustix::process::ioctl_tiocsctty(BorrowedFd::borrow_raw(0))?;
                Ok(())
            });
        }
        // The command, dropped here, holds this process's copies of the
        // terminal's side, which would keep it open.
        command.spawn()
    }

    /// What the terminal shows from now on, without its carriage returns,
    /// as its lines would read. It ends once no process holds the terminal
    /// any more. A thread reads the terminal, since a pseudo-terminal
    /// cannot be waited on everywhere as a socket can (macOS's kqueue does
    /// not report one), and it ends then too, or once the stream is
    /// dropped and the terminal shows more.
    pub fn output(&self) -> io::Result<tokio::net::UnixStream> {
        let main = self.main.try_clone()?;
        let (shown, mut sink) = UnixStream::pair()?;
        std::thread::spawn(move || {
            let mut buffer = [0; 8192];
            loop {
                let read = match rustix::io::read(&main, &mut buffer) {
                    Ok(0) => return,
                    Ok(read) => read,
                    Err(rustix::io::Errno::INTR) => continue,
                    // `EIO`: the last process that held the terminal let
                    // it go.
                    Err(_) => return,
                };
                let text: Vec<u8> = buffer[..read]
                    .iter()
                    .copied()
                    .filter(|byte| *byte != b'\r')
                    .collect();
                // The test no longer reads it.
                if sink.write_all(&text).is_err() {
                    return;
                }
            }
        });
        shown.set_nonblocking(true)?;
        tokio::net::UnixStream::from_std(shown)
    }

    /// Presses Ctrl-C: the terminal sends `SIGINT` to its foreground
    /// process group. It reaches nobody once no process holds the terminal
    /// any more, where macOS refuses the key with `EIO`.
    pub fn interrupt(&self) -> io::Result<()> {
        match rustix::io::write(&self.main, b"\x03") {
            Ok(_) | Err(rustix::io::Errno::IO) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

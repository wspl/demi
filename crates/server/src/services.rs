//! The two services, through systemd: an upgrade stops, starts and reloads
//! them, and nothing else. A start returns once the service reports ready or
//! has failed, since both units use `Type=notify`.

use std::{io, process::Command};

/// What an upgrade asks of the service manager.
pub trait Services {
    fn stop(&self, unit: &str) -> io::Result<()>;
    /// Starts `unit` and waits until it reports ready or fails.
    fn start(&self, unit: &str) -> io::Result<()>;
    /// Reads the units again and enables both.
    fn reload(&self) -> io::Result<()>;
    /// The last lines of `unit`'s log, for a failure's report.
    fn log(&self, unit: &str) -> String;
}

/// systemd's own commands.
pub struct Systemd;

impl Services for Systemd {
    fn stop(&self, unit: &str) -> io::Result<()> {
        systemctl(&["stop", unit])
    }

    fn start(&self, unit: &str) -> io::Result<()> {
        systemctl(&["start", unit])
    }

    fn reload(&self) -> io::Result<()> {
        systemctl(&["daemon-reload"])?;
        let mut enable = vec!["enable"];
        enable.extend(crate::layout::UNITS);
        systemctl(&enable)
    }

    fn log(&self, unit: &str) -> String {
        match Command::new("journalctl")
            .args(["-u", unit, "-n", "30", "--no-pager"])
            .output()
        {
            Ok(output) => String::from_utf8_lossy(&output.stdout).into_owned(),
            Err(error) => format!("the log of {unit} could not be read: {error}"),
        }
    }
}

fn systemctl(args: &[&str]) -> io::Result<()> {
    let status = Command::new("systemctl").args(args).status()?;
    if !status.success() {
        return Err(io::Error::other(format!(
            "systemctl {} failed: {status}",
            args.join(" ")
        )));
    }
    Ok(())
}

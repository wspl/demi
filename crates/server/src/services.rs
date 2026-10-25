//! The two services, through systemd: an upgrade stops, starts and reloads
//! them, and nothing else, and keeps the path unit of an administrator's
//! update running when its release has one. A start returns once the service reports ready or
//! has failed, since both units use `Type=notify`.
//!
//! While neither backend runs, the update screen answers the pages: the new
//! release's backend in its screen's mode, a transient unit with the
//! backend unit's user, capability and configuration.

use std::{io, process::Command};

/// The update screen's transient unit.
const SCREEN: &str = "demi-update-screen.service";

/// What an upgrade asks of the service manager.
pub trait Services: Sync {
    fn stop(&self, unit: &str) -> io::Result<()>;
    /// Starts `unit` and waits until it reports ready or fails.
    fn start(&self, unit: &str) -> io::Result<()>;
    /// Reads the units again and enables both, and runs the update's path
    /// unit when `upgrades` says its units are in place, else stops it.
    fn reload(&self, upgrades: bool) -> io::Result<()>;
    /// The last lines of `unit`'s log, for a failure's report.
    fn log(&self, unit: &str) -> String;
    /// Starts the backend `program` with the configuration `config` as the
    /// update screen of the steps in `record`, unless it runs.
    fn screen(&self, program: &std::path::Path, config: &std::path::Path, record: &std::path::Path) -> io::Result<()>;
    /// Stops the update screen, which frees the backend's address.
    fn end_screen(&self);
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

    fn reload(&self, upgrades: bool) -> io::Result<()> {
        // A path unit whose file went keeps running until it is stopped.
        if !upgrades {
            let _ = systemctl(&["disable", "--now", crate::layout::UPGRADE_PATH]);
        }
        systemctl(&["daemon-reload"])?;
        let mut enable = vec!["enable"];
        enable.extend(crate::layout::UNITS);
        systemctl(&enable)?;
        if upgrades {
            systemctl(&["enable", "--now", crate::layout::UPGRADE_PATH])?;
        }
        Ok(())
    }

    fn screen(&self, program: &std::path::Path, config: &std::path::Path, record: &std::path::Path) -> io::Result<()> {
        if Command::new("systemctl").args(["is-active", "--quiet", SCREEN]).status()?.success() {
            return Ok(());
        }
        let _ = Command::new("systemctl").args(["reset-failed", SCREEN]).status();
        let status = Command::new("systemd-run")
            .args([
                "--quiet",
                "--collect",
                &format!("--unit={SCREEN}"),
                "--property=User=demi",
                "--property=Group=demi",
                "--property=AmbientCapabilities=CAP_NET_BIND_SERVICE",
                "--property=CapabilityBoundingSet=CAP_NET_BIND_SERVICE",
            ])
            .arg(format!("--property=EnvironmentFile={}", config.display()))
            .arg(program)
            .arg("--update-screen")
            .arg(record)
            .status()?;
        if !status.success() {
            return Err(io::Error::other(format!("systemd-run {SCREEN} failed: {status}")));
        }
        Ok(())
    }

    fn end_screen(&self) {
        let _ = Command::new("systemctl").args(["stop", SCREEN]).status();
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

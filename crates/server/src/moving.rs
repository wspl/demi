//! A move of the server to a release (`upgrades.md` § Prepare, § Switch,
//! § Rollback), which the release moved to carries out with its own
//! knowledge: its programs check the configuration and import its image,
//! it knows which databases it migrates and its manager's state format.
//! Then, with each step in the journal first, both services stop, the
//! databases are copied or put back, `current` and the units change, and both
//! start; an upgrade whose services do not start returns to the release it
//! left.

use std::{fmt, io, process::Command};

use demi_machine_manager_protocol::STATE_FORMAT;
use semver::Version;

use crate::{
    data,
    gvisor,
    journal::{Data, Journal, Step},
    layout::{Layout, UNITS},
    services::Services,
    settings::Settings,
};

/// Why a move did not reach its release.
#[derive(Debug)]
pub enum Failure {
    /// The preparation or a step failed; for a step, the server is where
    /// the journal says, and the next run finishes the move.
    Failed(io::Error),
    /// The release's services did not start, and the server runs the
    /// release it left again.
    Returned {
        to: Version,
        from: Version,
        reason: io::Error,
        log: String,
    },
}

impl fmt::Display for Failure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Failed(error) => write!(formatter, "{error}"),
            Self::Returned {
                to,
                from,
                reason,
                log,
            } => write!(
                formatter,
                "{to} did not start ({reason}); the server runs {from} again, with its data as it was\n{log}"
            ),
        }
    }
}

impl From<io::Error> for Failure {
    fn from(error: io::Error) -> Self {
        Self::Failed(error)
    }
}

/// Prepares the move from the current release to `to`, this program's own,
/// while the current one serves, and carries it out.
pub fn start(layout: &Layout, services: &dyn Services, to: &Version) -> Result<(), Failure> {
    let from = layout.current_version()?;
    if &from == to {
        return Ok(());
    }
    let settings = Settings::read(&layout.config())?;
    let data = prepare(layout, &settings, &from, to)?;
    let journal = Journal {
        from,
        to: to.clone(),
        data,
        step: Step::Stopping,
    };
    carry_out(layout, services, journal)
}

/// Everything that can fail without stopping anything: the configuration,
/// gVisor, the image, the room for the copy, and the manager's state format.
fn prepare(
    layout: &Layout,
    settings: &Settings,
    from: &Version,
    to: &Version,
) -> io::Result<Data> {
    let bin = layout.release(to).join("bin");
    run(settings.command(&bin.join("demi-backend")).arg("--check-config"))?;
    let manager = bin.join("demi-machine-manager");
    run(settings.command(&manager).arg("--check-config"))?;
    gvisor::of_release(layout, to)?;
    let format = manager_format(&settings.manager_data()?)?;
    let backend = settings.backend_data()?;
    if to < from {
        if format.is_some_and(|format| format > STATE_FORMAT) {
            return Err(io::Error::other(format!(
                "{from} changed the machine manager's state format, so the server cannot return to {to}"
            )));
        }
        run(settings.command(&manager).arg("--import"))?;
        let restore = data::snapshot(&backend, to).exists();
        return Ok(if restore { Data::Restore } else { Data::Keep });
    }
    if format.is_some_and(|format| format < STATE_FORMAT) {
        println!("{to} changes the machine manager's state format: a rollback cannot return to {from}");
    }
    run(settings.command(&manager).arg("--import"))?;
    let migrating = data::migrating(&backend)?;
    if migrating.is_empty() {
        return Ok(Data::Keep);
    }
    let size = data::size(&backend, &migrating)?;
    if !data::room(&backend, size)? {
        return Err(io::Error::other(format!(
            "{} has no room for the {size} bytes of the databases {to} migrates",
            backend.display()
        )));
    }
    Ok(Data::Snapshot)
}

/// The format the manager's state directory records, if any.
fn manager_format(state: &std::path::Path) -> io::Result<Option<u32>> {
    match std::fs::read_to_string(state.join("format")) {
        Ok(text) => text
            .trim()
            .parse()
            .map(Some)
            .map_err(|_| io::Error::other(format!("{} holds no format", state.display()))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// Takes the journal's steps from the one it reached, each recorded before
/// it is taken, and removes the journal once the release runs.
pub fn carry_out(layout: &Layout, services: &dyn Services, mut journal: Journal) -> Result<(), Failure> {
    let path = layout.journal();
    let settings = Settings::read(&layout.config())?;
    let backend = settings.backend_data()?;
    for step in [Step::Stopping, Step::Data, Step::Switching, Step::Starting] {
        if step < journal.step {
            continue;
        }
        journal.record(&path, step)?;
        match step {
            Step::Stopping => {
                for unit in UNITS.iter().rev() {
                    services.stop(unit)?;
                }
            }
            Step::Data => match journal.data {
                Data::Keep => {}
                Data::Snapshot => {
                    let migrating = data::migrating(&backend)?;
                    data::take(&backend, &journal.from, &migrating)?;
                }
                Data::Restore => data::restore(&backend, &journal.to, &journal.from)?,
            },
            Step::Switching => {
                layout.point_at(&journal.to)?;
                layout.install_units(&journal.to)?;
                services.reload()?;
            }
            Step::Starting => {
                if let Err((unit, reason)) = start_all(services) {
                    let log = services.log(unit);
                    // A rollback that does not start stays, for the next run
                    // to try again: its release is the one to return to.
                    if journal.to < journal.from {
                        return Err(Failure::Failed(reason));
                    }
                    return_to_from(layout, services, &backend, &journal)?;
                    Journal::remove(&path)?;
                    return Err(Failure::Returned {
                        to: journal.to,
                        from: journal.from,
                        reason,
                        log,
                    });
                }
            }
        }
    }
    Journal::remove(&path)?;
    prune(layout, &backend, &journal)?;
    Ok(())
}

/// Starts both services in order; the unit that failed and why.
fn start_all(services: &dyn Services) -> Result<(), (&'static str, io::Error)> {
    for unit in UNITS {
        services.start(unit).map_err(|error| (unit, error))?;
    }
    Ok(())
}

/// Puts the release an upgrade left back, with the databases it copied.
fn return_to_from(
    layout: &Layout,
    services: &dyn Services,
    backend: &std::path::Path,
    journal: &Journal,
) -> io::Result<()> {
    for unit in UNITS.iter().rev() {
        services.stop(unit)?;
    }
    if journal.data == Data::Snapshot {
        data::restore(backend, &journal.from, &journal.to)?;
    }
    layout.point_at(&journal.from)?;
    layout.install_units(&journal.from)?;
    services.reload()?;
    start_all(services).map_err(|(unit, error)| {
        io::Error::other(format!("{} did not start again either: {unit}: {error}", journal.from))
    })
}

/// Keeps the two releases of the move with their gVisor versions, and the
/// one snapshot a rollback may need: an upgrade's copy of the release it
/// left, or a rollback's databases it moved aside.
fn prune(layout: &Layout, backend: &std::path::Path, journal: &Journal) -> io::Result<()> {
    for version in layout.versions()? {
        if version != journal.to && version != journal.from {
            std::fs::remove_dir_all(layout.release(&version))?;
        }
    }
    gvisor::prune(layout, &[&journal.to, &journal.from])?;
    let kept = match journal.data {
        Data::Keep => Vec::new(),
        Data::Snapshot => vec![journal.from.to_string()],
        Data::Restore => vec![format!("abandoned-{}", journal.from)],
    };
    data::prune(backend, &kept)
}

/// Runs a program of the release, which reports its own failure.
fn run(command: &mut Command) -> io::Result<()> {
    let status = command.status()?;
    if !status.success() {
        return Err(io::Error::other(format!(
            "{} failed: {status}",
            command.get_program().to_string_lossy()
        )));
    }
    Ok(())
}

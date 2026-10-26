//! A move of the server to a release (`upgrades.md` § Prepare, § Switch, §
//! Rollback), which the release moved to carries out with its own knowledge: it
//! knows which databases it migrates and its manager's state format.
//!
//! The move has these steps. From step 2 on, each step is recorded in the
//! journal first.
//!
//! 1. The release's programs check the configuration and import its image.
//! 2. Both services stop.
//! 3. The databases are copied or put back.
//! 4. `current` and the units change.
//! 5. Both services start.
//!
//! An upgrade whose services do not start returns to the release it left.
//!
//! Every step of the move is one of the update's steps (`notices`), which the
//! running backend hears and the backend's data directory keeps: the
//! preparation's, Settings › Updates's, and from the restart on the update
//! screen's. While no backend runs, the new release's backend answers the
//! pages in its update screen's mode, from those steps, so every page sees
//! how far the move is: from the old backend's stop until the new one
//! starts, and again while a return runs.

use std::{fmt, io, process::Command};

use demi_machine_manager_protocol::{DRAIN_RECORD, DrainRecord, STATE_FORMAT, runtime::RuntimeRelease};
use demi_runner_protocol::release::{
    RestartStepKind, SkipReason, StepProgress, StepState, StepSummary, UpgradeStepKind,
};
use semver::Version;

use crate::{
    data,
    gvisor,
    journal::{Data, Journal, Step},
    layout::{BACKEND, Layout, MANAGER, UNITS},
    notices::Notices,
    progress,
    runners,
    services::Services,
    settings::Settings,
};

/// Why a move did not reach its release.
#[derive(Debug)]
pub enum Failure {
    /// The preparation or a step failed; for a step, the server is where
    /// the journal says, and the next run finishes the move.
    Failed(io::Error),
    /// A step after the backend's stop failed: nothing serves but the
    /// update screen, which shows the failed step, until the next run
    /// finishes the move.
    Stopped(io::Error),
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
            Self::Failed(error) | Self::Stopped(error) => write!(formatter, "{error}"),
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

impl Failure {
    /// Why the move did not reach its release, in a line for a page.
    pub fn reason(&self) -> String {
        match self {
            Self::Failed(error) | Self::Stopped(error) => error.to_string(),
            Self::Returned { to, reason, .. } => format!("{to} did not start: {reason}"),
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
pub fn start(layout: &Layout, services: &dyn Services, to: &Version, notices: &mut Notices) -> Result<(), Failure> {
    let from = layout.current_version()?;
    if &from == to {
        return Ok(());
    }
    notices.moving(&from, to);
    // A move without the record of a fetch found the release unpacked, as a
    // rollback does.
    for kind in [UpgradeStepKind::Download, UpgradeStepKind::Unpack] {
        notices.skip_waiting(kind, SkipReason::Installed);
    }
    let settings = Settings::read(&layout.config())?;
    let data = prepare(layout, &settings, &from, to, notices)?;
    let journal = Journal {
        from,
        to: to.clone(),
        data,
        step: Step::Stopping,
    };
    carry_out(layout, services, journal, notices)
}

/// Everything that can fail without stopping anything: the configuration,
/// gVisor, the image, the room for the copy, and the manager's state format.
fn prepare(
    layout: &Layout,
    settings: &Settings,
    from: &Version,
    to: &Version,
    notices: &mut Notices,
) -> io::Result<Data> {
    // The paired devices download `to`'s runner while the preparation goes
    // on, and nothing waits for them.
    let socket = settings
        .manage_socket()
        .unwrap_or_else(|| layout.manage_socket());
    runners::start_prefetch(&socket, &layout.release(to))?;
    let bin = layout.release(to).join("bin");
    notices.start(UpgradeStepKind::CheckConfiguration);
    run(settings.command(&bin.join("demi-backend")).arg("--check-config"))?;
    let manager = bin.join("demi-machine-manager");
    run(settings.command(&manager).arg("--check-config"))?;
    let format = manager_format(&settings.manager_data()?)?;
    if to < from && format.is_some_and(|format| format > STATE_FORMAT) {
        return Err(io::Error::other(format!(
            "{from} changed the machine manager's state format, so the server cannot return to {to}"
        )));
    }
    notices.done(UpgradeStepKind::CheckConfiguration, StepSummary::None);
    if layout.gvisor().join(RuntimeRelease::pinned().version()).exists() {
        notices.step(UpgradeStepKind::DownloadGvisor, StepState::Skipped { why: SkipReason::Installed });
    } else {
        notices.start(UpgradeStepKind::DownloadGvisor);
        gvisor::of_release(layout, to)?;
        notices.done(UpgradeStepKind::DownloadGvisor, StepSummary::None);
    }
    if to > from && format.is_some_and(|format| format < STATE_FORMAT) {
        println!("{to} changes the machine manager's state format: a rollback cannot return to {from}");
    }
    install_image(layout, settings, to, &manager, notices)?;
    let backend = settings.backend_data()?;
    notices.start(UpgradeStepKind::CheckDatabases);
    let data = if to < from {
        if data::snapshot(&backend, to).exists() { Data::Restore } else { Data::Keep }
    } else {
        let migrating = data::migrating(&backend)?;
        if migrating.is_empty() {
            Data::Keep
        } else {
            let size = data::size(&backend, &migrating)?;
            if !data::room(&backend, size)? {
                return Err(io::Error::other(format!(
                    "{} has no room for the {size} bytes of the databases {to} migrates",
                    backend.display()
                )));
            }
            notices.done(
                UpgradeStepKind::CheckDatabases,
                StepSummary::Count {
                    count: u32::try_from(migrating.len()).unwrap_or(u32::MAX),
                },
            );
            Data::Snapshot
        }
    };
    if data != Data::Snapshot {
        notices.step(UpgradeStepKind::CheckDatabases, StepState::Skipped { why: SkipReason::NoChanges });
    }
    Ok(data)
}

/// The new manager imports the release's image beside the running one; a
/// base it has already is installed.
fn install_image(
    layout: &Layout,
    settings: &Settings,
    to: &Version,
    manager: &std::path::Path,
    notices: &mut Notices,
) -> io::Result<()> {
    let bases = settings.manager_data()?.join("images").join("bases");
    let before = entries(&bases)?;
    notices.start(UpgradeStepKind::InstallImage);
    run(settings.command(manager).arg("--import"))?;
    if entries(&bases)? == before {
        notices.step(UpgradeStepKind::InstallImage, StepState::Skipped { why: SkipReason::Installed });
    } else {
        let archive = layout.release(to).join("image").join("rootfs.tar.zst");
        let bytes = std::fs::metadata(archive).map_or(0, |metadata| metadata.len());
        notices.done(UpgradeStepKind::InstallImage, StepSummary::Bytes { bytes });
    }
    Ok(())
}

/// The names in `directory`, none when it does not exist.
fn entries(directory: &std::path::Path) -> io::Result<std::collections::BTreeSet<std::ffi::OsString>> {
    match std::fs::read_dir(directory) {
        Ok(read) => read.map(|entry| entry.map(|entry| entry.file_name())).collect(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Default::default()),
        Err(error) => Err(error),
    }
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
pub fn carry_out(
    layout: &Layout,
    services: &dyn Services,
    mut journal: Journal,
    notices: &mut Notices,
) -> Result<(), Failure> {
    let path = layout.journal();
    let settings = Settings::read(&layout.config())?;
    let backend = settings.backend_data()?;
    let screen = Screen {
        program: layout.release(&journal.to).join("bin/demi-backend"),
        config: layout.config(),
        record: notices.record_path(),
    };
    // A move taken again after its stop finds no backend: the screen
    // answers the pages until the new one starts.
    let mut stopped = journal.step > Step::Stopping;
    if stopped {
        screen.up(services);
    }
    let taken = take_steps(layout, services, &mut journal, notices, &settings, &backend, &screen, &mut stopped);
    match taken {
        Err(Failure::Failed(error)) if stopped => {
            screen.up(services);
            Err(Failure::Stopped(error))
        }
        Err(failure) => Err(failure),
        Ok(()) => {
            notices.finished();
            Journal::remove(&path)?;
            prune(layout, &backend, &journal)?;
            Ok(())
        }
    }
}

/// The journal's steps from the one it reached; `stopped` says whether the
/// backend no longer runs.
fn take_steps(
    layout: &Layout,
    services: &dyn Services,
    journal: &mut Journal,
    notices: &mut Notices,
    settings: &Settings,
    backend: &std::path::Path,
    screen: &Screen,
    stopped: &mut bool,
) -> Result<(), Failure> {
    let path = layout.journal();
    for step in [Step::Stopping, Step::Data, Step::Switching, Step::Starting] {
        if step < journal.step {
            continue;
        }
        journal.record(&path, step)?;
        match step {
            Step::Stopping => {
                // Every page shows the update's screen from here; the backend
                // answers once they had the time to hear it.
                notices.start(UpgradeStepKind::Restart);
                notices.start_restart(RestartStepKind::StopDemi);
                services.stop(BACKEND)?;
                *stopped = true;
                notices.done_restart(RestartStepKind::StopDemi, StepSummary::None);
                screen.up(services);
                save_clouds(services, notices)?;
            }
            Step::Data => {
                notices.start_restart(RestartStepKind::KeepDatabases);
                match journal.data {
                    Data::Keep => notices.restart(
                        RestartStepKind::KeepDatabases,
                        StepState::Skipped { why: SkipReason::NoChanges },
                    ),
                    Data::Snapshot => {
                        let migrating = data::migrating(backend)?;
                        let mut copied = 0;
                        data::take(backend, &journal.from, &migrating, &mut |done, total| {
                            copied = total;
                            notices.progress_restart(
                                RestartStepKind::KeepDatabases,
                                StepProgress::Bytes {
                                    done,
                                    total,
                                    per_second: None,
                                    left_seconds: None,
                                },
                            );
                        })?;
                        notices.done_restart(RestartStepKind::KeepDatabases, StepSummary::Bytes { bytes: copied });
                    }
                    Data::Restore => {
                        data::restore(backend, &journal.to, &journal.from)?;
                        notices.done_restart(RestartStepKind::KeepDatabases, StepSummary::None);
                    }
                }
            }
            Step::Switching => {
                // A return that did not finish leaves this step to take
                // again, towards the release the move goes to.
                notices.returning(false);
                notices.start_restart(RestartStepKind::Switch);
                layout.point_at(&journal.to)?;
                layout.install_units(&journal.to)?;
                services.reload(layout.upgrade_units())?;
                notices.done_restart(RestartStepKind::Switch, StepSummary::None);
            }
            Step::Starting => {
                // The backend reports ready once its data is up to date,
                // which may take minutes: meanwhile it says how far it is.
                let watch = settings.backend_listen().map(progress::Watch::start);
                let started = start_all(services, screen, notices);
                if let Some(watch) = watch {
                    watch.stop();
                }
                if let Err((unit, reason)) = started {
                    let log = services.log(unit);
                    // A rollback that does not start stays, for the next run
                    // to try again: its release is the one to return to.
                    if journal.to < journal.from {
                        return Err(Failure::Failed(reason));
                    }
                    notices.failed_step(&format!("{} did not start", journal.to), true);
                    // The return undoes step 4, so the journal goes back to
                    // it: a return that does not finish leaves the next run
                    // to switch to the release again, never to start the
                    // one the return pointed `current` at.
                    journal.record(&path, Step::Switching)?;
                    return_to_from(layout, services, backend, journal, screen)?;
                    *stopped = false;
                    Journal::remove(&path)?;
                    return Err(Failure::Returned {
                        to: journal.to.clone(),
                        from: journal.from.clone(),
                        reason,
                        log,
                    });
                }
            }
        }
    }
    Ok(())
}

/// Stops the manager, which saves every running Cloud, telling how many it
/// has saved of how many as its record of the drain says.
fn save_clouds(services: &dyn Services, notices: &mut Notices) -> io::Result<()> {
    /// How often the drain's record is read.
    const EVERY: std::time::Duration = std::time::Duration::from_millis(500);
    let _ = std::fs::remove_file(DRAIN_RECORD);
    notices.start_restart(RestartStepKind::SaveClouds);
    let mut saving = 0;
    let stopped = std::thread::scope(|scope| {
        let stopping = scope.spawn(|| services.stop(MANAGER));
        let mut last = None;
        while !stopping.is_finished() {
            let read = std::fs::read(DRAIN_RECORD)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<DrainRecord>(&bytes).ok());
            if let Some(record) = read
                && last != Some(record)
            {
                saving = record.saving;
                notices.progress_restart(
                    RestartStepKind::SaveClouds,
                    StepProgress::Clouds {
                        done: record.saved,
                        total: record.saving,
                    },
                );
                last = Some(record);
            }
            std::thread::sleep(EVERY);
        }
        stopping.join().unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    });
    stopped?;
    notices.done_restart(RestartStepKind::SaveClouds, StepSummary::Count { count: saving });
    Ok(())
}

/// The new release's backend in its update screen's mode, which answers the
/// pages from the update's steps while no backend runs.
struct Screen {
    program: std::path::PathBuf,
    config: std::path::PathBuf,
    record: Option<std::path::PathBuf>,
}

impl Screen {
    /// Starts the screen; one that does not start leaves the pages showing
    /// what they read last.
    fn up(&self, services: &dyn Services) {
        if let Some(record) = &self.record
            && let Err(error) = services.screen(&self.program, &self.config, record)
        {
            eprintln!("The update screen did not start: {error}");
        }
    }

    fn down(&self, services: &dyn Services) {
        if self.record.is_some() {
            services.end_screen();
        }
    }
}

/// Starts the manager, then the backend, which takes the address from the
/// screen; the unit that failed and why.
fn start_all(
    services: &dyn Services,
    screen: &Screen,
    notices: &mut Notices,
) -> Result<(), (&'static str, io::Error)> {
    notices.start_restart(RestartStepKind::StartManager);
    services.start(MANAGER).map_err(|error| (MANAGER, error))?;
    notices.done_restart(RestartStepKind::StartManager, StepSummary::None);
    screen.down(services);
    // The backend takes its start, and its data's steps, from here.
    notices.start_restart(RestartStepKind::StartDemi);
    services.start(BACKEND).map_err(|error| (BACKEND, error))?;
    Ok(())
}

/// Puts the release an upgrade left back, with the databases it copied;
/// the screen answers the pages meanwhile.
fn return_to_from(
    layout: &Layout,
    services: &dyn Services,
    backend: &std::path::Path,
    journal: &Journal,
    screen: &Screen,
) -> io::Result<()> {
    for unit in UNITS.iter().rev() {
        services.stop(unit)?;
    }
    screen.up(services);
    if journal.data == Data::Snapshot {
        data::restore(backend, &journal.from, &journal.to)?;
    }
    layout.point_at(&journal.from)?;
    layout.install_units(&journal.from)?;
    services.reload(layout.upgrade_units())?;
    let started = services
        .start(MANAGER)
        .and_then(|()| {
            screen.down(services);
            services.start(BACKEND)
        });
    started.map_err(|error| {
        io::Error::other(format!(
            "{} did not start again either: {error}; the next demi-server command switches to {} again",
            journal.from, journal.to
        ))
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

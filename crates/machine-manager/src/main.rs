//! The `demi-machine-manager` executable: the one place the manager is
//! assembled.
//!
//! Before it serves a request, the manager does these in order:
//!
//! 1. It validates its configuration.
//! 2. It checks that it runs as root in a private mount namespace, that its
//!    programs resolve and that `runsc` reports exactly the pinned version.
//! 3. It takes its two locks. A second manager on the same state or runtime
//!    directory is refused, and the error names the path.
//! 4. It recovers a namespace an earlier manager saved (`namespace`), unless
//!    that manager left its running sandboxes to this one (`handover`): then
//!    the manager entered that namespace before it started a thread, and takes
//!    each sandbox it recorded over.
//! 5. With the resource limits on, it requires the cgroup v2 CPU, memory and
//!    PID controllers and enables them for the sandboxes. With them off, it
//!    touches no cgroup.
//! 6. It stops and saves every device left behind but those it took over
//!    (`recovery`), ends the processes of its unit that no sandbox owns, then
//!    migrates its records when they are in an earlier release's format
//!    (`storage::state`).
//! 7. It installs the network policy, with the slots of the sandboxes it took
//!    over, imports the configured base unless an upgrade imported it already
//!    (`--import`) and collects the bases nothing needs.
//! 8. It pins its own mount namespace, then mounts its directory of network
//!    namespaces, which the pin's bind on the host would otherwise cover
//!    (`network::netns`); the namespace of a manager that left its sandboxes
//!    is pinned and mounted already.
//! 9. It probes with a small image that storage can be mounted, frozen and
//!    copied.
//! 10. It opens its socket.
//!
//! Readiness goes to systemd last, whose unit has no start timeout, since a
//! first import or a large recovery can take minutes.
//!
//! On SIGTERM the manager stops accepting connections, stops and saves every
//! device, and then fails the requests still waiting, so none can start a
//! sandbox after the drain. An upgrade's leave request makes it leave its
//! running sandboxes to the next manager instead (`handover`): it records
//! them and keeps its namespace pinned. It releases the namespace handle
//! only if the drain succeeded. Otherwise the service's stop-post command runs the recovery
//! (`--recover`), limited to fencing and saving, since a stopped service
//! neither resolves names nor installs firewall rules. Stopping has no timeout
//! either: the drain and the recovery check and publish filesystems, and
//! killing a check midway does more harm than waiting.

#[cfg(not(target_os = "linux"))]
fn main() -> std::process::ExitCode {
    eprintln!("demi-machine-manager: the Cloud manager runs only on Linux");
    std::process::ExitCode::FAILURE
}

/// Lines of the log waiting for the server's fan-out, which takes them at
/// once; the queue is bounded so that memory is.
#[cfg(target_os = "linux")]
const LOG_LINES: usize = 256;

#[cfg(target_os = "linux")]
fn main() -> std::process::ExitCode {
    // A panic leaves its report on standard error
    // (`crates/xtask/src/native/build.rs`).
    demi_shared_cli::install_panic_hook();
    use demi_shared_types::{LogOrigin, log_layer::LineLayer};
    use tracing_subscriber::{filter::LevelFilter, fmt, prelude::*};

    let config = match demi_machine_manager::config::Config::from_env() {
        Ok(config) => config,
        Err(demi_machine_manager::config::ConfigError::Invalid(error)) => error.exit(),
        Err(error) => {
            eprintln!("demi-machine-manager: {error}");
            return std::process::ExitCode::FAILURE;
        }
    };
    // The journal records the service's standard error and the time. The
    // events of `info` and above also go to the backend as lines of the log,
    // on each connection the backend has open; a line made while none is
    // open stays in the journal alone.
    let (lines, forwarded) = tokio::sync::mpsc::channel(LOG_LINES);
    tracing_subscriber::registry()
        .with(
            fmt::layer()
                .with_writer(std::io::stderr)
                .without_time()
                .with_target(false),
        )
        .with(
            LineLayer::new(LogOrigin::MachineManager, move |line| {
                // Logging never holds up what it describes: a line the
                // queue has no room for stays in the journal alone.
                let _ = lines.try_send(line);
            })
            .with_filter(LevelFilter::INFO),
        )
        .init();
    // Before any thread starts, so that every thread works in the
    // namespace: the one a manager that left its sandboxes saved, which
    // holds their storage, or a copy of this process's own.
    let mut adopting = false;
    if config.mode == demi_machine_manager::config::Mode::Serve {
        let entered = match demi_machine_manager::handover::read_left() {
            Ok(Some(_)) => {
                let namespace = demi_machine_manager::namespace::SavedNamespace::new(&config);
                // A namespace that holds another state directory's
                // sandboxes is refused, as its recovery refuses it.
                match namespace
                    .owned()
                    .and_then(|owned| owned.then(|| demi_machine_manager::handover::enter_saved_namespace(namespace.handle())).transpose())
                {
                    Ok(Some(())) => true,
                    Ok(None) => {
                        tracing::warn!(event = "machine_manager.namespace_not_entered", "the saved namespace is gone or holds another state directory's sandboxes, which are not taken over");
                        false
                    }
                    Err(error) => {
                        tracing::warn!(event = "machine_manager.namespace_not_entered", %error, "the sandboxes the manager before this one left are fenced instead of taken over");
                        false
                    }
                }
            }
            Ok(None) => false,
            Err(error) => {
                tracing::warn!(event = "machine_manager.left_record_unreadable", %error, "the sandboxes the manager before this one left are fenced instead of taken over");
                false
            }
        };
        adopting = entered;
        if !entered
            && let Err(error) = demi_machine_manager::namespace::number_after_host()
        {
            tracing::error!(
                event = "machine_manager.exited",
                error_chain = %error,
                "the machine manager ended with an error"
            );
            return std::process::ExitCode::FAILURE;
        }
    }
    // One thread owns the manager's state.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build_local(tokio::runtime::LocalOptions::default())
        .expect("the manager's runtime");
    let result = runtime.block_on(service::run(config, forwarded, adopting));
    // Every task the manager owns has ended; blocking jobs that remain belong
    // to nothing that waits for them.
    runtime.shutdown_background();
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(
                event = "machine_manager.exited",
                error_chain = %demi_machine_manager::server::chain(&*error),
                "the machine manager ended with an error"
            );
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(target_os = "linux")]
mod service {
    use std::{error::Error, rc::Rc};

    use demi_machine_manager::{
        blocking,
        config::{Config, Mode},
        handover,
        lock::{self, ManagerLock},
        manager::{Core, Manager},
        namespace::SavedNamespace,
        preflight, recovery,
        sandbox::cgroup,
        server::{self, Socket},
        storage::{base, durable, room, state, store::ImageStore},
        tools::Tools,
    };
    use demi_shared_types::LogLine;
    use tokio::signal::unix::{SignalKind, signal};
    use tokio::sync::mpsc::Receiver;

    type Failure = Box<dyn Error>;

    /// Imports the configured release's image beside a running manager, as
    /// an upgrade does before it stops that manager
    /// (`crates/server/src/main.rs`), and prints its base version. Bases
    /// are immutable directories published under their own names, and a
    /// running manager reads none it was not configured with, so the
    /// import takes none of the manager's locks and needs no private
    /// namespace. It replaces the base an earlier import ahead of an update
    /// left, which no manager ran, so one waits ahead at most.
    async fn import(config: Config) -> Result<(), Failure> {
        let runsc = config.runsc.clone();
        let tools = blocking::run(move |_| Tools::resolve(&runsc)).await?;
        let bases = ImageStore::new(config.images()).bases();
        let data = config.data.clone();
        blocking::run(move |off| durable::create_private(off, &data)).await?;
        let base = base::import(&tools, &config.image, &bases).await?;
        base::imported_ahead(&config.images(), &base).await?;
        println!("{base}");
        Ok(())
    }

    /// Checks what a start checks before it serves, changing nothing that
    /// outlives the check (`installation.md` § The steps): each check prints
    /// its name as it passes, and the first that fails ends the run.
    async fn check_host(config: Config) -> Result<(), Failure> {
        blocking::run(preflight::require_private_namespace).await?;
        println!("ok: privileges");
        let runsc = config.runsc.clone();
        let tools = blocking::run(move |_| Tools::resolve(&runsc)).await?;
        let core = Rc::new(Core::new(config, tools));
        preflight::require_runsc(&core).await?;
        println!("ok: programs and runsc");
        if core.config.limits.is_some() {
            cgroup::check().await?;
            println!("ok: cgroup v2 controllers");
        }
        let (data, runtime) = (core.config.data.clone(), core.config.runtime().to_owned());
        blocking::run(move |off| -> std::io::Result<()> {
            durable::create_private(off, &data)?;
            durable::create_private(off, &runtime)
        })
        .await?;
        let (working, images) = (core.config.working(), core.config.images());
        blocking::run(move |off| preflight::require_one_filesystem(off, &working, &images)).await?;
        preflight::probe_storage(&core).await?;
        println!("ok: storage");
        core.network.check().await?;
        println!("ok: network");
        preflight::probe_runtime(&core).await?;
        println!("ok: gVisor");
        Ok(())
    }

    /// Deaths and operations' ends waiting for the server's fan-out, which
    /// takes them at once; each queue is bounded so that memory is.
    const DEATHS: usize = 64;

    /// Serves until a stop; `adopting` when this process entered the saved
    /// namespace of a manager that left its sandboxes (`handover`), which it
    /// takes over.
    pub async fn run(config: Config, lines: Receiver<LogLine>, adopting: bool) -> Result<(), Failure> {
        if config.mode == Mode::CheckConfig {
            return Ok(());
        }
        preflight::require_root()?;
        let data = config.data.clone();
        blocking::run(move |off| room::require_mounted(off, &data)).await?;
        if config.mode == Mode::Import {
            return import(config).await;
        }
        if config.mode == Mode::CheckHost {
            return check_host(config).await;
        }
        blocking::run(preflight::require_private_namespace).await?;
        let runsc = config.runsc.clone();
        let tools = blocking::run(move |_| Tools::resolve(&runsc)).await?;
        let core = Rc::new(Core::new(config, tools));
        preflight::require_runsc(&core).await?;
        let (data, runtime) = (core.config.data.clone(), core.config.runtime().to_owned());
        blocking::run(move |off| -> std::io::Result<()> {
            durable::create_private(off, &data)?;
            durable::create_private(off, &runtime)
        })
        .await?;
        state::require_format(&core.config.data).await?;
        if core.config.mode == Mode::RecoverNamespace {
            // The manager that started this process holds the locks and
            // waits for it, inside the namespace it recovers.
            let data = core.config.data.clone();
            blocking::run(move |off| lock::verify_inherited(off, &data)).await?;
            preflight::release_probes(&core.config.data).await?;
            recovery::fence_and_save(&core, &[]).await?;
            return Ok(());
        }
        let (data, runtime) = (core.config.data.clone(), core.config.runtime().to_owned());
        let lock = blocking::run(move |off| ManagerLock::acquire(off, &data, &runtime)).await?;
        let namespace = SavedNamespace::new(&core.config);
        let left = handover::read_left()?;
        // The namespace of a manager that left its sandboxes holds them for
        // the next manager: the stop-post recovery leaves it alone.
        if core.config.mode == Mode::Recover && left.is_some() {
            tracing::info!(event = "machine_manager.recovery_skipped", "the manager left its sandboxes to the next one");
            return Ok(());
        }
        let left = left.filter(|_| adopting);
        // Taking over reads records and asks runsc, and touches no disk
        // image, so the sweep below knows what to keep before anything does.
        let mut adopted = Vec::new();
        let mut owners = Vec::new();
        for sandbox in left.iter().flat_map(|left| &left.sandboxes) {
            match handover::take_over(&core, sandbox).await {
                Ok(Some(taken)) => {
                    tracing::info!(event = "cloud_runtime.taken_over", host_id = %sandbox.device, "a Cloud the manager before this one left running was taken over");
                    adopted.push((sandbox.device.clone(), taken.sandbox));
                    owners.push(taken.owner);
                }
                Ok(None) => tracing::info!(event = "cloud_runtime.ended_meanwhile", host_id = %sandbox.device, "a Cloud the manager before this one left had ended; it is saved"),
                Err(error) => tracing::warn!(event = "cloud_runtime.take_over_failed", host_id = %sandbox.device, %error, "a Cloud the manager before this one left could not be taken over; it is saved"),
            }
        }
        namespace.refuse_other()?;
        // What systemd's stop used to end, which no record owns, such as an
        // `e2fsck` of a manager killed while it repaired an image: gone
        // before any recovery writes that image again.
        handover::sweep(owners).await?;
        if left.is_none() {
            namespace.recover(&lock).await?;
        }
        preflight::release_probes(&core.config.data).await?;
        match core.config.limits {
            Some(_) => cgroup::prepare().await?,
            None => tracing::warn!(
                event = "machine_manager.limits_off",
                "DEMI_MANAGED_LIMITS=off: Clouds run without CPU, memory or PID limits"
            ),
        }
        let (working, images) = (core.config.working(), core.config.images());
        blocking::run(move |off| preflight::require_one_filesystem(off, &working, &images)).await?;
        let running: Vec<_> = adopted.iter().map(|(device, _)| device.clone()).collect();
        recovery::fence_and_save(&core, &running).await?;
        if core.config.mode == Mode::Recover {
            return Ok(());
        }
        // The recovery above fenced every recorded sandbox but those taken
        // over, which keep running.
        let slots: Vec<_> = adopted.iter().map(|(_, sandbox)| sandbox.slot().clone()).collect();
        core.network.prepare(&slots).await?;
        let base = base::import(&core.tools, &core.config.image, &core.store.bases()).await?;
        let programs: Vec<_> = left
            .iter()
            .flat_map(|left| &left.sandboxes)
            .filter(|sandbox| running.contains(&sandbox.device))
            .map(|sandbox| sandbox.programs.clone())
            .collect();
        base::retain(&core.config.images(), &core.config.working(), &base, &programs).await?;
        // The saved namespace a manager that left its sandboxes pinned is
        // this one, with its directory of network namespaces mounted.
        if left.is_none() {
            namespace.pin().await?;
            core.network.mount_namespace_directory().await?;
        }
        preflight::probe_storage(&core).await?;
        let socket_path = core.config.socket.clone();
        let socket = Socket::bind(&socket_path).await?;
        let (deaths, received) = tokio::sync::mpsc::channel(DEATHS);
        let (changes, changed) = tokio::sync::mpsc::channel(DEATHS);
        let manager = Rc::new(Manager::new(core.clone(), base, deaths, changes));
        for (device, sandbox) in adopted {
            manager.adopt(device, sandbox);
        }
        handover::remove_left().await?;
        let mut terminate = signal(SignalKind::terminate())?;
        let mut interrupt = signal(SignalKind::interrupt())?;
        // Readiness for systemd's Type=notify; without a notify socket this
        // does nothing.
        sd_notify::notify(&[sd_notify::NotifyState::Ready])?;
        let limits = if core.config.limits.is_some() {
            "on"
        } else {
            "off"
        };
        tracing::info!(
            event = "machine_manager.ready",
            socket = %socket_path.display(),
            limits,
            "gVisor/systrap is ready"
        );
        let stopping = async move {
            tokio::select! {
                _ = terminate.recv() => {}
                _ = interrupt.recv() => {}
            }
        };
        let in_flight = server::serve(socket, manager.clone(), received, changed, lines, stopping).await;
        // An upgrade that keeps the state format asks the manager to leave
        // its sandboxes to the next one rather than save them.
        if handover::take_request().await? {
            let left = manager.leave().await;
            in_flight.wait().await;
            handover::write_left(left?).await?;
            tracing::info!(event = "machine_manager.left", "the manager left its running Clouds to the next one");
            // The saved namespace holds the sandboxes for the next manager.
            drop(lock);
            return Ok(());
        }
        let drained = manager.close().await;
        in_flight.wait().await;
        drained?;
        // Only a successful drain releases the handle; otherwise the
        // service's stop-post recovery works through it.
        namespace.release().await?;
        drop(lock);
        Ok(())
    }
}

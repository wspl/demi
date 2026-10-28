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
//! 4. It recovers a namespace an earlier manager saved (`namespace`).
//! 5. With the resource limits on, it requires the cgroup v2 CPU, memory and
//!    PID controllers and enables them for the sandboxes. With them off, it
//!    touches no cgroup.
//! 6. It stops and saves every device left behind (`recovery`), then migrates
//!    its records when they are in an earlier release's format
//!    (`storage::state`).
//! 7. It installs the network policy, imports the configured base unless an
//!    upgrade imported it already (`--import`) and collects the bases nothing
//!    needs.
//! 8. It pins its own mount namespace, then mounts its directory of network
//!    namespaces, which the pin's bind on the host would otherwise cover
//!    (`network::netns`).
//! 9. It probes with a small image that storage can be mounted, frozen and
//!    copied.
//! 10. It opens its socket.
//!
//! Readiness goes to systemd last, whose unit has no start timeout, since a
//! first import or a large recovery can take minutes.
//!
//! On SIGTERM the manager stops accepting connections, stops and saves every
//! device, and then fails the requests still waiting, so none can start a
//! sandbox after the drain. It releases the namespace handle only if the drain
//! succeeded. Otherwise the service's stop-post command runs the recovery
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
    // Before any thread starts, so that every thread works in the copy.
    if config.mode == demi_machine_manager::config::Mode::Serve
        && let Err(error) = demi_machine_manager::namespace::number_after_host()
    {
        tracing::error!(
            event = "machine_manager.exited",
            error_chain = %error,
            "the machine manager ended with an error"
        );
        return std::process::ExitCode::FAILURE;
    }
    // One thread owns the manager's state.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build_local(tokio::runtime::LocalOptions::default())
        .expect("the manager's runtime");
    let result = runtime.block_on(service::run(config, forwarded));
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
        lock::{self, ManagerLock},
        manager::{Core, Manager},
        namespace::SavedNamespace,
        preflight, recovery,
        sandbox::cgroup,
        server::{self, Socket},
        storage::{base, durable, room, shared, state, store::ImageStore},
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

    pub async fn run(config: Config, lines: Receiver<LogLine>) -> Result<(), Failure> {
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
            shared::unmount_left(&core).await?;
            return Ok(());
        }
        let (data, runtime) = (core.config.data.clone(), core.config.runtime().to_owned());
        let lock = blocking::run(move |off| ManagerLock::acquire(off, &data, &runtime)).await?;
        let namespace = SavedNamespace::new(&core.config);
        namespace.recover(&lock).await?;
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
        recovery::fence_and_save(&core, &[]).await?;
        // No sandbox runs: no shared folder is in use.
        shared::unmount_left(&core).await?;
        if core.config.mode == Mode::Recover {
            return Ok(());
        }
        // The recovery above fenced every recorded sandbox, so none runs.
        core.network.prepare(&[]).await?;
        let base = base::import(&core.tools, &core.config.image, &core.store.bases()).await?;
        base::retain(&core.config.images(), &core.config.working(), &base).await?;
        namespace.pin().await?;
        core.network.mount_namespace_directory().await?;
        preflight::probe_storage(&core).await?;
        let socket_path = core.config.socket.clone();
        let socket = Socket::bind(&socket_path).await?;
        let (deaths, received) = tokio::sync::mpsc::channel(DEATHS);
        let (changes, changed) = tokio::sync::mpsc::channel(DEATHS);
        let manager = Rc::new(Manager::new(core.clone(), base, deaths, changes));
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

//! The `demi-machine-manager` executable: the one place the manager is assembled
//! (`managed-hosts.md` § Startup and recovery).

#[cfg(not(target_os = "linux"))]
fn main() -> std::process::ExitCode {
    eprintln!("demi-machine-manager: the Cloud manager runs only on Linux");
    std::process::ExitCode::FAILURE
}

#[cfg(target_os = "linux")]
fn main() -> std::process::ExitCode {
    // A panic leaves its report on standard error (`builds-and-releases.md`
    // § Build profiles).
    demi_shared_cli::install_panic_hook();
    use tracing_subscriber::{fmt, prelude::*};

    let config = match demi_machine_manager::config::Config::from_env() {
        Ok(config) => config,
        Err(demi_machine_manager::config::ConfigError::Invalid(error)) => error.exit(),
        Err(error) => {
            eprintln!("demi-machine-manager: {error}");
            return std::process::ExitCode::FAILURE;
        }
    };
    // The journal records the service's standard error and the time; the
    // messages name their part of the manager themselves.
    tracing_subscriber::registry()
        .with(
            fmt::layer()
                .with_writer(std::io::stderr)
                .without_time()
                .with_target(false),
        )
        .init();
    // One thread owns the manager's state (`concurrency.md` § Machine manager).
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build_local(tokio::runtime::LocalOptions::default())
        .expect("the manager's runtime");
    let result = runtime.block_on(service::run(config));
    // Every task the manager owns has ended; blocking jobs that remain belong
    // to nothing that waits for them.
    runtime.shutdown_background();
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(
                "demi-machine-manager: {}",
                demi_machine_manager::server::chain(&*error)
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
        storage::{base, durable, state, store::ImageStore},
        tools::Tools,
    };
    use tokio::signal::unix::{SignalKind, signal};

    type Failure = Box<dyn Error>;

    /// Imports the configured release's image beside a running manager, as
    /// an upgrade does before it stops that manager (`upgrades.md`
    /// § Prepare), and prints its base version. Bases are immutable
    /// directories published under their own names, and a running manager
    /// reads none it was not configured with, so the import takes none of
    /// the manager's locks and needs no private namespace.
    async fn import(config: Config) -> Result<(), Failure> {
        let runsc = config.runsc.clone();
        let tools = blocking::run(move |_| Tools::resolve(&runsc)).await?;
        let bases = ImageStore::new(config.images()).bases();
        let data = config.data.clone();
        blocking::run(move |off| durable::create_private(off, &data)).await?;
        let base = base::import(&tools, &config.image, &bases).await?;
        println!("{base}");
        Ok(())
    }

    /// Checks what a start checks before it serves, changing nothing that
    /// outlives the check (`installation.md` § The steps): each check prints
    /// its name as it passes, and the first that fails ends the run.
    async fn check_host(config: Config) -> Result<(), Failure> {
        blocking::run(preflight::require_private_namespace).await?;
        blocking::run(preflight::require_sys_resource).await?;
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

    /// Deaths waiting for the server's fan-out, which takes them at once.
    const DEATHS: usize = 64;

    pub async fn run(config: Config) -> Result<(), Failure> {
        if config.mode == Mode::CheckConfig {
            return Ok(());
        }
        preflight::require_root()?;
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
        namespace.recover(&lock).await?;
        preflight::release_probes(&core.config.data).await?;
        match core.config.limits {
            Some(_) => cgroup::prepare().await?,
            None => tracing::warn!(
                "demi-machine-manager: DEMI_MANAGED_LIMITS=off: Clouds run without CPU, memory or PID limits"
            ),
        }
        let (working, images) = (core.config.working(), core.config.images());
        blocking::run(move |off| preflight::require_one_filesystem(off, &working, &images)).await?;
        recovery::fence_and_save(&core, &[]).await?;
        if core.config.mode == Mode::Recover {
            return Ok(());
        }
        core.network.prepare().await?;
        let base = base::import(&core.tools, &core.config.image, &core.store.bases()).await?;
        base::retain(&core.config.images(), &core.config.working(), &base).await?;
        namespace.pin().await?;
        preflight::probe_storage(&core).await?;
        let socket_path = core.config.socket.clone();
        let socket = Socket::bind(&socket_path).await?;
        let (deaths, received) = tokio::sync::mpsc::channel(DEATHS);
        let manager = Rc::new(Manager::new(core.clone(), base, deaths));
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
            "demi-machine-manager: gVisor/systrap ready at {}, resource limits {limits}",
            socket_path.display()
        );
        let stopping = async move {
            tokio::select! {
                _ = terminate.recv() => {}
                _ = interrupt.recv() => {}
            }
        };
        let in_flight = server::serve(socket, manager.clone(), received, stopping).await;
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

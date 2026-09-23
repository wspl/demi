//! The `demi-machines` executable: the one place the manager is assembled
//! (`managed-hosts.md` § Startup and recovery).

#[cfg(not(target_os = "linux"))]
fn main() -> std::process::ExitCode {
    eprintln!("demi-machines: the Cloud manager runs only on Linux");
    std::process::ExitCode::FAILURE
}

#[cfg(target_os = "linux")]
fn main() -> std::process::ExitCode {
    use tracing_subscriber::{fmt, prelude::*};

    let config = match demi_machines::config::Config::from_env() {
        Ok(config) => config,
        Err(demi_machines::config::ConfigError::Invalid(error)) => error.exit(),
        Err(error) => {
            eprintln!("demi-machines: {error}");
            return std::process::ExitCode::FAILURE;
        }
    };
    // The journal records the service's standard error.
    tracing_subscriber::registry()
        .with(fmt::layer().with_writer(std::io::stderr).without_time())
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
            tracing::error!("demi-machines: {}", demi_machines::server::chain(&*error));
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(target_os = "linux")]
mod service {
    use std::{error::Error, rc::Rc};

    use demi_machines::{
        blocking,
        config::{Config, Mode},
        lock::{self, ManagerLock},
        manager::{Core, Manager},
        namespace::SavedNamespace,
        preflight, recovery,
        sandbox::cgroup,
        server::{self, Socket},
        storage::{base, durable},
        tools::Tools,
    };
    use tokio::signal::unix::{SignalKind, signal};

    type Failure = Box<dyn Error>;

    /// Deaths waiting for the server's fan-out, which takes them at once.
    const DEATHS: usize = 64;

    pub async fn run(config: Config) -> Result<(), Failure> {
        preflight::require_root()?;
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
        if core.config.mode == Mode::RecoverNamespace {
            // The manager that started this process holds the locks and
            // waits for it, inside the namespace it recovers.
            let data = core.config.data.clone();
            blocking::run(move |off| lock::verify_inherited(off, &data)).await?;
            preflight::release_probes(&core.config.data).await?;
            recovery::fence_and_save(&core).await?;
            return Ok(());
        }
        let (data, runtime) = (core.config.data.clone(), core.config.runtime().to_owned());
        let lock = blocking::run(move |off| ManagerLock::acquire(off, &data, &runtime)).await?;
        let namespace = SavedNamespace::new(&core.config);
        namespace.recover(&lock).await?;
        preflight::release_probes(&core.config.data).await?;
        cgroup::prepare().await?;
        let (working, images) = (core.config.working(), core.config.images());
        blocking::run(move |off| preflight::require_one_filesystem(off, &working, &images)).await?;
        recovery::fence_and_save(&core).await?;
        if core.config.mode == Mode::Recover {
            return Ok(());
        }
        core.network.prepare().await?;
        let base = base::import(&core.tools, &core.config.image, &core.store.bases()).await?;
        namespace.pin().await?;
        preflight::probe_storage(&core).await?;
        let socket_path = core.config.socket.clone().expect("serving has a socket");
        let socket = Socket::bind(&socket_path).await?;
        let (deaths, received) = tokio::sync::mpsc::channel(DEATHS);
        let manager = Rc::new(Manager::new(core.clone(), base, deaths));
        let mut terminate = signal(SignalKind::terminate())?;
        let mut interrupt = signal(SignalKind::interrupt())?;
        // Readiness for systemd's Type=notify; without a notify socket this
        // does nothing.
        sd_notify::notify(&[sd_notify::NotifyState::Ready])?;
        tracing::info!("demi-machines: gVisor/systrap ready at {}", socket_path.display());
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

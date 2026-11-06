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
//! 5. With the resource limits on, it requires the cgroup CPU, memory and PID
//!    controllers, of cgroup v2 or v1, and enables v2's for the sandboxes.
//!    With them off, it touches no cgroup.
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
    use tracing_subscriber::{
        filter::{LevelFilter, Targets},
        fmt,
        prelude::*,
    };

    let config = match demi_machine_manager::config::Invocation::from_env() {
        Ok(demi_machine_manager::config::Invocation::Manage(config)) => config,
        Ok(demi_machine_manager::config::Invocation::FetchGvisor(directory)) => return fetch_gvisor(&directory),
        Err(demi_machine_manager::config::ConfigError::Invalid(error)) => error.exit(),
        Err(error) => {
            eprintln!("demi-machine-manager: {error}");
            return std::process::ExitCode::FAILURE;
        }
    };
    // The journal records the service's standard error and the time. The
    // events of `info` and above also go to the backend as lines of the log,
    // on each connection the backend has open; a line made while none is
    // open stays in the journal alone. The journal takes the manager's own
    // `debug` events too, but only `info` and above of its libraries: the
    // HTTP client's `trace` events of one gVisor download are 130,000 lines.
    let (lines, forwarded) = tokio::sync::mpsc::channel(LOG_LINES);
    tracing_subscriber::registry()
        .with(
            fmt::layer()
                .with_writer(std::io::stderr)
                .without_time()
                .with_target(false)
                .with_filter(Targets::new().with_target("demi_machine_manager", LevelFilter::DEBUG).with_default(LevelFilter::INFO)),
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
    // Why this manager cannot run gVisor, as far as its namespace tells:
    // only root makes the copy of the mount namespace gVisor's sandboxes
    // need.
    let mut namespace = Ok(());
    if !rustix::process::geteuid().is_root() {
        namespace = Err("the machine manager does not run as root".to_owned());
    } else if config.mode == demi_machine_manager::config::Mode::Serve {
        let entered = match demi_machine_manager::handover::read_left() {
            // Only gVisor sandboxes live in the saved namespace; Simple
            // Isolation Clouds a manager left need none.
            Ok(Some(left)) if !left.sandboxes.is_empty() => {
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
            Ok(_) => false,
            Err(error) => {
                tracing::warn!(event = "machine_manager.left_record_unreadable", %error, "the sandboxes the manager before this one left are fenced instead of taken over");
                false
            }
        };
        adopting = entered;
        if !entered
            && let Err(error) = demi_machine_manager::namespace::number_after_host()
        {
            namespace = Err(format!("the mount namespace cannot be copied: {error}"));
        }
    }
    // One thread owns the manager's state.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build_local(tokio::runtime::LocalOptions::default())
        .expect("the manager's runtime");
    let result = runtime.block_on(service::run(config, forwarded, adopting, namespace));
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

/// Fetches the pinned gVisor release into `directory` and prints the
/// version's directory, for `demi-server runtime`.
#[cfg(target_os = "linux")]
fn fetch_gvisor(directory: &std::path::Path) -> std::process::ExitCode {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("the fetch's runtime");
    let cancel = tokio_util::sync::CancellationToken::new();
    match runtime.block_on(demi_machine_manager::gvisor_release::fetch(directory, &cancel, |_, _| {})) {
        Ok(fetched) => {
            println!("{}", fetched.display());
            std::process::ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("demi-machine-manager: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(target_os = "linux")]
mod service {
    use std::{error::Error, net::Ipv4Addr, rc::Rc};

    use demi_machine_manager::{
        blocking,
        config::{Config, Mode},
        handover,
        lock::{self, ManagerLock},
        manager::{Core, Manager},
        namespace::SavedNamespace,
        network::CloudNetwork,
        preflight, recovery,
        runtime::simple,
        sandbox::{cgroup, runsc::Runsc},
        server::{self, Socket},
        storage::{base, durable, room, state, store::ImageStore},
        tools::{Tool, Tools},
    };
    use demi_machine_manager_protocol::{
        Download, Enforces, Isolation, LeftSandbox, LeftSimple, NetworkRules, Offer, RuntimeOffer, Unavailable,
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
        let tools = resolve(&config).await?;
        let bases = ImageStore::new(config.images()).bases();
        let data = config.data.clone();
        blocking::run(move |off| durable::create_private(off, &data)).await?;
        let base = base::import(&tools, &config.image, &bases).await?;
        base::imported_ahead(&config.images(), &base).await?;
        println!("{base}");
        Ok(())
    }

    async fn resolve(config: &Config) -> Result<Tools, Failure> {
        let (runsc, proot) = (config.runsc.clone(), config.proot.clone());
        Ok(blocking::run(move |_| Tools::resolve(&runsc, &proot)).await?)
    }

    /// Why a manager cannot run gVisor.
    enum NoGvisor {
        /// It lacks only the pinned release, which it fetches when the
        /// master chooses gVisor.
        Release,
        Missing(String),
    }

    impl NoGvisor {
        fn reason(&self) -> String {
            match self {
                Self::Release => "gVisor's release isn't downloaded".into(),
                Self::Missing(reason) => reason.clone(),
            }
        }
    }

    impl From<String> for NoGvisor {
        fn from(reason: String) -> Self {
            Self::Missing(reason)
        }
    }

    /// The resolvers gVisor's Clouds use, or why this manager cannot run
    /// gVisor: the checks of a start that change nothing but make the
    /// working and image directories, after the namespace `main` made. A
    /// manager that would fetch the pinned release lacks only that when it
    /// passes every other check.
    async fn gvisor(config: &Config, tools: &Tools, namespace: Result<(), String>) -> Result<Vec<Ipv4Addr>, NoGvisor> {
        namespace?;
        let dns = config.gvisor_resolvers()?;
        blocking::run(preflight::require_private_namespace).await.map_err(|error| error.to_string())?;
        let release = match tools.gvisor() {
            Ok(()) => true,
            Err(missing) if missing.only(Tool::Runsc) && config.gvisor_directory().is_some() => false,
            Err(missing) => return Err(missing.to_string().into()),
        };
        if release {
            let runsc = Runsc::new(tools.clone(), config.runtime(), config.limits.is_some());
            preflight::require_runsc(&runsc).await.map_err(|error| error.to_string())?;
        }
        if config.limits.is_some() {
            cgroup::check().await.map_err(|error| error.to_string())?;
        }
        let network = CloudNetwork::new(config.subnet, dns.clone(), tools.path(Tool::Nft).to_owned());
        network.check().await.map_err(|error| error.to_string())?;
        let (working, images) = (config.working(), config.images());
        blocking::run(move |off| preflight::require_one_filesystem(off, &working, &images))
            .await
            .map_err(|error| error.to_string())?;
        match release {
            true => Ok(dns),
            false => Err(NoGvisor::Release),
        }
    }

    /// Why a manager can run neither runtime: Simple Isolation's `error` and
    /// gVisor's `reason`.
    fn no_runtime(error: &impl std::fmt::Display, reason: &str) -> Failure {
        format!("no Cloud can run here: {error}. gVisor isn't available: {reason}").into()
    }

    /// Checks what a start checks before it serves, changing nothing that
    /// outlives the check (`installation.md` § The steps): each check prints
    /// its name as it passes. A machine passes when it runs either runtime,
    /// as a start serves then, and the run says why it cannot run the other.
    async fn check_host(config: Config, namespace: Result<(), String>) -> Result<(), Failure> {
        let tools = resolve(&config).await?;
        println!("ok: programs");
        let data = config.data.clone();
        blocking::run(move |off| durable::create_private(off, &data)).await?;
        let simple = simple::Simple::new(&config).probe(&config.data.join("simple-probe")).await;
        match &simple {
            Ok(()) => println!("ok: Simple Isolation"),
            Err(error) => println!("Simple Isolation isn't available: {error}"),
        }
        let probed = match gvisor(&config, &tools, namespace).await {
            Ok(dns) => {
                println!("ok: network");
                probe_gvisor(config, tools, dns).await.map_err(|error| error.to_string())
            }
            Err(no) => Err(no.reason()),
        };
        match (probed, simple) {
            (Ok(()), _) => Ok(()),
            (Err(reason), Ok(())) => {
                println!("gVisor isn't available: {reason}");
                Ok(())
            }
            (Err(reason), Err(error)) => Err(no_runtime(&error, &reason)),
        }
    }

    /// Runs a gVisor sandbox on the storage a start uses, for the host
    /// check, removing what it made.
    async fn probe_gvisor(config: Config, tools: Tools, dns: Vec<Ipv4Addr>) -> Result<(), Failure> {
        let core = Rc::new(Core::new(config, tools, Ok(dns)));
        let runtime = core.config.runtime().to_owned();
        blocking::run(move |off| durable::create_private(off, &runtime)).await?;
        preflight::probe_storage(&core).await?;
        println!("ok: storage");
        preflight::probe_runtime(&core).await?;
        println!("ok: gVisor");
        Ok(())
    }

    /// Deaths and operations' ends waiting for the server's fan-out, which
    /// takes them at once; each queue is bounded so that memory is.
    const DEATHS: usize = 64;

    /// What gVisor enforces, where it runs and where it runs once fetched.
    const GVISOR_ENFORCES: Enforces = Enforces {
        checkpoints: true,
        storage_limits: true,
        network_policy: true,
    };

    /// Serves until a stop; `adopting` when this process entered the saved
    /// namespace of a manager that left its sandboxes (`handover`), which it
    /// takes over, and `namespace` why it could not make the namespace
    /// gVisor needs, if it could not.
    pub async fn run(config: Config, lines: Receiver<LogLine>, adopting: bool, namespace: Result<(), String>) -> Result<(), Failure> {
        if config.mode == Mode::CheckConfig {
            return Ok(());
        }
        let data = config.data.clone();
        blocking::run(move |off| room::require_mounted(off, &data)).await?;
        if config.mode == Mode::Import {
            return import(config).await;
        }
        if config.mode == Mode::CheckHost {
            return check_host(config, namespace).await;
        }
        let tools = resolve(&config).await?;
        let (gvisor, release_missing) = match gvisor(&config, &tools, namespace).await {
            Ok(dns) => (Ok(dns), false),
            Err(no) => (Err(no.reason()), matches!(no, NoGvisor::Release)),
        };
        if let Err(reason) = &gvisor {
            if config.mode == Mode::RecoverNamespace {
                return Err(reason.clone().into());
            }
            tracing::warn!(event = "machine_manager.gvisor_unavailable", reason, "gVisor isn't available; Clouds run on Simple Isolation");
        }
        let core = Rc::new(Core::new(config, tools, gvisor));
        let runs_gvisor = core.gvisor.is_ok();
        // Only root has the runtime directory, where an upgrade asks a
        // manager to leave its Clouds and the manager records those it left.
        let root = rustix::process::geteuid().is_root();
        let (data, runtime) = (core.config.data.clone(), core.config.runtime().to_owned());
        blocking::run(move |off| -> std::io::Result<()> {
            durable::create_private(off, &data)?;
            if root {
                durable::create_private(off, &runtime)?;
            }
            Ok(())
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
        let (data, runtime) = (core.config.data.clone(), runs_gvisor.then(|| core.config.runtime().to_owned()));
        let lock = blocking::run(move |off| ManagerLock::acquire(off, &data, runtime.as_deref())).await?;
        let namespace = SavedNamespace::new(&core.config);
        let left = if root { handover::read_left()? } else { None };
        // The namespace of a manager that left its sandboxes holds them for
        // the next manager: the stop-post recovery leaves it alone.
        if core.config.mode == Mode::Recover && left.is_some() {
            tracing::info!(event = "machine_manager.recovery_skipped", "the manager left its sandboxes to the next one");
            return Ok(());
        }
        // The gVisor sandboxes a manager left are taken over only from
        // inside the saved namespace that holds them, which this process
        // entered when `adopting`; its Simple Isolation Clouds need none.
        let saved = adopting && left.is_some();
        let left_sandboxes: &[LeftSandbox] = match &left {
            Some(left) if adopting => &left.sandboxes,
            _ => &[],
        };
        let left_simple: &[LeftSimple] = left.as_ref().map_or(&[], |left| &left.simple);
        // Taking over reads records and asks runsc, and touches no disk
        // image, so the sweep below knows what to keep before anything does.
        let mut adopted = Vec::new();
        let mut owners = Vec::new();
        for sandbox in left_sandboxes {
            match handover::take_over(&core, sandbox).await {
                Ok(Some(taken)) => {
                    tracing::info!(event = "cloud_runtime.taken_over", device_id = %sandbox.device, "a Cloud the manager before this one left running was taken over");
                    adopted.push((sandbox.device.clone(), taken.sandbox));
                    owners.push(taken.owner);
                }
                Ok(None) => tracing::info!(event = "cloud_runtime.ended_meanwhile", device_id = %sandbox.device, "a Cloud the manager before this one left had ended; it is saved"),
                Err(error) => tracing::warn!(event = "cloud_runtime.take_over_failed", device_id = %sandbox.device, %error, "a Cloud the manager before this one left could not be taken over; it is saved"),
            }
        }
        let mut adopted_simple = Vec::new();
        for left in left_simple {
            match simple::Process::take_over(&core.simple, left).await {
                Ok(Some(process)) => {
                    tracing::info!(event = "cloud_runtime.taken_over", device_id = %left.device, "a Cloud the manager before this one left running was taken over");
                    adopted_simple.push((left.device.clone(), process));
                }
                Ok(None) => tracing::info!(event = "cloud_runtime.ended_meanwhile", device_id = %left.device, "a Cloud the manager before this one left had ended"),
                Err(error) => tracing::warn!(event = "cloud_runtime.take_over_failed", device_id = %left.device, %error, "a Cloud the manager before this one left could not be taken over; it is stopped"),
            }
        }
        if runs_gvisor {
            namespace.refuse_other()?;
        }
        // What systemd's stop used to end, which no record owns, such as an
        // `e2fsck` of a manager killed while it repaired an image: gone
        // before any recovery writes that image again.
        let proots = adopted_simple.iter().map(|(_, process)| process.pid()).collect();
        handover::sweep(owners, proots).await?;
        if runs_gvisor && !saved {
            namespace.recover(&lock).await?;
        }
        let running_simple: Vec<_> = adopted_simple.iter().map(|(device, _)| device.clone()).collect();
        core.simple.prepare(&core.config.data).await?;
        core.simple.recover(&running_simple).await?;
        let running: Vec<_> = adopted.iter().map(|(device, _)| device.clone()).collect();
        if runs_gvisor {
            preflight::release_probes(&core.config.data).await?;
            match core.config.limits {
                Some(_) => cgroup::prepare().await?,
                None => tracing::warn!(
                    event = "machine_manager.limits_off",
                    "DEMI_MANAGED_LIMITS=off: Clouds run without CPU, memory or PID limits"
                ),
            }
            recovery::fence_and_save(&core, &running).await?;
        }
        if core.config.mode == Mode::Recover {
            return Ok(());
        }
        // The recovery above fenced every recorded sandbox but those taken
        // over, which keep running. They reach the internet alone until the
        // backend, as it connects, names the network rules.
        if let Ok(gvisor) = &core.gvisor {
            let slots: Vec<_> = adopted.iter().filter_map(|(_, sandbox)| sandbox.admitted()).collect();
            gvisor.network.prepare(&slots, &NetworkRules::default()).await?;
        }
        let base = base::import(&core.tools, &core.config.image, &core.bases()).await?;
        let programs: Vec<_> = left_sandboxes
            .iter()
            .filter(|sandbox| running.contains(&sandbox.device))
            .map(|sandbox| sandbox.programs.clone())
            .collect();
        let trees = core.simple.trees.clone();
        let pinned = blocking::run(move |off| trees.bases(off)).await?;
        base::retain(&core.config.images(), &core.config.working(), &base, &programs, &pinned).await?;
        let probed = core.simple.probe(&core.config.data.join("simple-probe")).await;
        match (&probed, &core.gvisor) {
            (Ok(()), _) => {}
            (Err(error), Ok(_)) => tracing::warn!(event = "machine_manager.simple_unavailable", %error, "Simple Isolation isn't available; Clouds run on gVisor"),
            (Err(error), Err(reason)) => return Err(no_runtime(error, reason)),
        }
        if runs_gvisor {
            // The saved namespace a manager that left its sandboxes pinned is
            // this one, with its directory of network namespaces mounted.
            if !saved {
                namespace.pin().await?;
                core.gvisor().network.mount_namespace_directory().await?;
            }
            preflight::probe_storage(&core).await?;
        }
        let socket_path = core.config.socket.clone();
        let socket = Socket::bind(&socket_path).await?;
        let (deaths, received) = tokio::sync::mpsc::channel(DEATHS);
        let (changes, changed) = tokio::sync::mpsc::channel(DEATHS);
        let offers = vec![
            RuntimeOffer {
                isolation: Isolation::Simple,
                offer: match &probed {
                    Ok(()) => Offer::Runs(Enforces {
                        checkpoints: false,
                        storage_limits: false,
                        network_policy: false,
                    }),
                    Err(error) => Offer::Unavailable(Unavailable { reason: error.to_string() }),
                },
            },
            RuntimeOffer {
                isolation: Isolation::Gvisor,
                offer: match &core.gvisor {
                    Ok(_) => Offer::Runs(GVISOR_ENFORCES),
                    Err(_) if release_missing => Offer::AfterDownload {
                        enforces: GVISOR_ENFORCES,
                        download: Download::Waiting,
                    },
                    Err(reason) => Offer::Unavailable(Unavailable { reason: reason.clone() }),
                },
            },
        ];
        let manager = Manager::new(core.clone(), base, offers, deaths, changes);
        for (device, sandbox) in adopted {
            manager.adopt(device, sandbox);
        }
        for (device, process) in adopted_simple {
            manager.adopt_simple(device, process);
        }
        if root {
            handover::remove_left().await?;
        }
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
        let runtimes = match (&core.gvisor, &probed) {
            (Ok(_), Ok(())) => "gVisor/systrap and Simple Isolation are",
            (Ok(_), Err(_)) => "gVisor/systrap is",
            (Err(_), _) => "Simple Isolation is",
        };
        tracing::info!(
            event = "machine_manager.ready",
            socket = %socket_path.display(),
            limits,
            "{runtimes} ready"
        );
        let stopping = async move {
            tokio::select! {
                _ = terminate.recv() => {}
                _ = interrupt.recv() => {}
            }
        };
        let offers = manager.watch_offers();
        let in_flight = server::serve(socket, manager.clone(), received, changed, offers, lines, stopping).await;
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
        if runs_gvisor {
            namespace.release().await?;
        }
        drop(lock);
        Ok(())
    }
}

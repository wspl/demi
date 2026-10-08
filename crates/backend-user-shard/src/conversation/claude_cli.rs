//! Demi's Claude Code CLI on the user's Cloud (`claude-code.md` § The
//! package, § Where it runs, § Accounts and sign-in, § What the user sees):
//! the versions the Cloud has and the install of the vendor's newest,
//! through the `demi.claude-code` package; the placement that starts a
//! provider's process there, each in a configuration directory of its own
//! that goes with it; the acting user's Cloud as the machine a sign-in runs
//! on; and the installs no conversation asked for, whose outcome the
//! settings page shows.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::rc::{Rc, Weak};
use std::sync::{Mutex, MutexGuard, PoisonError};

use bytes::Bytes;
use demi_backend_cloud::access::MachineAccess;
use demi_backend_database::StorageError;
use demi_backend_providers::llm::claude_releases::ReleaseError;
use demi_backend_remote_host::{AttachedArtifact, RemoteHost, ServiceCallError, ServiceRequest};
use demi_backend_runners::command_context::{command_context, provider_context};
use demi_command_package_claude_code_protocol::{
    Installed, Operation, PACKAGE, Release, Reply, Status,
};
use demi_command_protocol::{
    ArtifactLocation, ArtifactUrl, CommandCaller, CommandContext, PackageArtifact,
};
use demi_host_interface::{
    Host as _, HostError, MkdirOptions, Process, ProcessEnd, RmOptions, SpawnRequest,
};
use demi_provider_claude_code::{
    AccountMachine, AccountWork, CliSite, ConfigDir, Placed, Placement, StartError,
};
use demi_web_api_protocol::ids::{ConversationId, ProviderId, UserId};
use demi_web_api_protocol::providers::{CliInstall, CliMachine};
use futures_util::FutureExt as _;
use futures_util::future::{BoxFuture, LocalBoxFuture, Shared};
use serde::de::DeserializeOwned;
use tokio_util::sync::CancellationToken;

use crate::services::Services;
use crate::shard::{Shard, Shards};

/// The most an operation of the package answers: one JSON document.
const MAX_ANSWER_BYTES: usize = 1024 * 1024;

/// Why the Cloud has no usable CLI, with the version that was wanted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CliError {
    version: Option<String>,
    reason: String,
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.version {
            Some(version) => write!(
                f,
                "Claude Code {version} could not be installed: {}",
                self.reason
            ),
            None => write!(f, "Claude Code could not be installed: {}", self.reason),
        }
    }
}

impl std::error::Error for CliError {}

impl From<ReleaseError> for CliError {
    fn from(error: ReleaseError) -> Self {
        Self {
            version: None,
            reason: error.to_string(),
        }
    }
}

/// What a provider's process is for (`claude-code.md` § Where it runs); it
/// names the command context the package's operations run under.
#[derive(Debug, Clone)]
pub(crate) enum ProcessWork {
    /// A request of the conversation.
    Conversation(ConversationId),
    /// Work of the provider entry that belongs to no conversation: an
    /// install, or **Test connection**.
    Account(ProviderId),
}

/// A Cloud the CLI is wanted on: a Host there, its home, and the context
/// the package's operations run under.
#[derive(Clone)]
struct CliTarget {
    host: RemoteHost,
    home: String,
    context: CommandContext,
}

/// The parent of the processes' configuration directories on the Cloud:
/// outside every workspace, and cleared when the Cloud stops or resets, so
/// a directory whose process the stop ended goes with it.
const CONFIG_PARENT: &str = "/tmp";

/// The user's Cloud as the placement of a provider's process: each start
/// wakes or admits the Cloud through machine access, makes sure it has the
/// CLI, makes the process's configuration directory, private to the
/// Cloud's user, and holds the Cloud's admission only until the process
/// started; a process that is retained holds none afterwards.
pub(crate) struct CloudPlacement {
    shard: Weak<Shard>,
    work: ProcessWork,
}

impl CloudPlacement {
    /// The placement a provider's process starts with.
    pub fn placement(shard: Weak<Shard>, work: ProcessWork) -> Rc<dyn Placement> {
        Rc::new(Self { shard, work })
    }
}

impl Placement for CloudPlacement {
    fn start<'a>(
        &'a self,
        spawn: &'a dyn Fn(&CliSite) -> SpawnRequest,
    ) -> LocalBoxFuture<'a, Result<Placed, StartError>> {
        Box::pin(async move {
            let shard = self
                .shard
                .upgrade()
                .ok_or_else(|| StartError("The backend is shutting down".into()))?;
            let access = shard
                .cloud_shard()
                .machine_access()
                .await
                .map_err(|error| StartError(error.to_string()))?;
            let executable = shard
                .cli_executable_at(&access, &self.work)
                .await
                .map_err(StartError)?;
            let run_dir = run_dir(&access.home);
            access
                .host
                .fs()
                .mkdir(&run_dir, MkdirOptions { recursive: true })
                .await
                .map_err(|error| {
                    StartError(format!(
                        "Claude Code's directory {run_dir} could not be made on the Cloud: {error}"
                    ))
                })?;
            // Dropping the directory's handle before the process started,
            // as when the start is given up, removes it.
            let mut config = CloudConfigDir {
                shard: self.shard.clone(),
                host: access.host.clone(),
                path: format!("{CONFIG_PARENT}/demi-claude-{}", uuid::Uuid::new_v4().simple()),
                exit: None,
                removed: false,
            };
            config.make().await.map_err(|error| {
                StartError(format!(
                    "Claude Code's configuration directory could not be made on the Cloud: {error}"
                ))
            })?;
            let site = CliSite {
                executable,
                run_dir,
                config_dir: config.path.clone(),
            };
            let process = access
                .host
                .process()
                .spawn(spawn(&site))
                .await
                .map_err(|error| {
                    StartError(format!(
                        "Claude Code could not be started on the Cloud: {error}"
                    ))
                })?;
            let exit = process.exit.shared();
            config.exit = Some(exit.clone());
            Ok(Placed {
                process: Process {
                    exit: exit.boxed_local(),
                    ..process
                },
                config: Box::new(config),
            })
        })
    }
}

/// A process's configuration directory on the Cloud, removed once the
/// process ended.
struct CloudConfigDir {
    shard: Weak<Shard>,
    host: RemoteHost,
    path: String,
    /// The process's end; none until it started.
    exit: Option<Shared<LocalBoxFuture<'static, ProcessEnd>>>,
    removed: bool,
}

impl CloudConfigDir {
    /// Makes the directory, new and private to the Cloud's user: a path
    /// that exists already is refused rather than taken over.
    async fn make(&self) -> Result<(), HostError> {
        let fs = self.host.fs();
        fs.mkdir(&self.path, MkdirOptions { recursive: false })
            .await?;
        fs.chmod(&self.path, 0o700).await
    }

    /// Removes the directory once its process ended.
    fn removal(&mut self) -> LocalBoxFuture<'static, ()> {
        self.removed = true;
        let host = self.host.clone();
        let path = self.path.clone();
        let exit = self.exit.take();
        Box::pin(async move {
            if let Some(exit) = exit {
                exit.await;
            }
            let options = RmOptions {
                recursive: true,
                force: true,
            };
            // A Cloud that stopped meanwhile cleared the directory with its
            // `/tmp`; anything else is a directory left behind, said here.
            if let Err(error) = host.fs().rm(&path, options).await {
                tracing::warn!(path, "Claude Code's configuration directory was not removed: {error}");
            }
        })
    }
}

impl ConfigDir for CloudConfigDir {
    fn read<'a>(&'a self, name: &'a str) -> LocalBoxFuture<'a, Result<Bytes, HostError>> {
        Box::pin(async move { self.host.fs().read_file(&format!("{}/{name}", self.path)).await })
    }

    fn remove(mut self: Box<Self>) -> LocalBoxFuture<'static, ()> {
        self.removal()
    }
}

impl Drop for CloudConfigDir {
    /// A directory nobody removed goes once its process ended, as a task of
    /// the shard, which shutdown waits for.
    fn drop(&mut self) {
        if self.removed {
            return;
        }
        let removal = self.removal();
        if let Some(shard) = self.shard.upgrade() {
            shard.tasks().spawn_local(removal);
        }
    }
}

/// Where a provider's process runs on a Cloud whose home is `home`.
fn run_dir(home: &str) -> String {
    format!("{home}/.demi/claude/run")
}

/// The acting user's Cloud as the machine a sign-in runs on, reached
/// through the user's shard from any thread (`claude-code.md` § Where it
/// runs).
pub struct ShardMachine {
    shards: Shards,
    user: UserId,
}

impl ShardMachine {
    pub fn new(shards: Shards, user: UserId) -> Self {
        Self { shards, user }
    }
}

impl AccountMachine for ShardMachine {
    fn run(&self, entry: String, work: AccountWork) -> BoxFuture<'static, Result<(), StartError>> {
        let shards = self.shards.clone();
        let user = self.user.clone();
        Box::pin(async move {
            let entry = ProviderId::try_from(entry).map_err(|error| StartError(error.to_string()))?;
            shards
                .of(&user)
                .call(move |shard, abandoned| async move {
                    let work_of = ProcessWork::Account(entry);
                    let placement = CloudPlacement::placement(Rc::downgrade(&shard), work_of);
                    work(placement, abandoned).await;
                })
                .await
                .map_err(|error| StartError(error.to_string()))
        })
    }
}

/// The CLI work of one user's shard: the versions being installed beside
/// the one in use.
#[derive(Default)]
pub(crate) struct ClaudeCli {
    upgrading: RefCell<HashSet<String>>,
}

impl Shard {
    /// The CLI's executable on the Cloud `access` holds, installed first
    /// when the Cloud has none.
    async fn cli_executable_at(
        &self,
        access: &MachineAccess,
        work: &ProcessWork,
    ) -> Result<String, String> {
        let control = &self.services().control;
        let context = match work {
            ProcessWork::Conversation(id) => {
                command_context(control, self.user(), id, CommandCaller::User {}).await
            }
            ProcessWork::Account(provider) => {
                provider_context(control, self.user(), provider).await
            }
        };
        let context =
            context.map_err(|error| format!("The command context could not be read: {error}"))?;
        let target = CliTarget {
            host: access.host.clone(),
            home: access.home.clone(),
            context,
        };
        self.cli_executable(&target)
            .await
            .map_err(|error| error.to_string())
    }

    /// The executable to start on `target`: the wanted version when the
    /// Cloud has it, else the newest it has while the wanted one installs
    /// beside it. Only a Cloud with none waits for an install.
    async fn cli_executable(&self, target: &CliTarget) -> Result<String, CliError> {
        let wanted = self.services().claude_releases.latest(false).await?;
        let installed = installed(self.services(), target).await?;
        let usable = installed
            .iter()
            .find(|installed| installed.version == wanted.version)
            .or_else(|| installed.first());
        let Some(usable) = usable else {
            return ensure(self.services(), target, &wanted).await;
        };
        if usable.version != wanted.version {
            self.upgrade_cli(target.clone(), wanted);
        }
        Ok(usable.path.to_string_lossy().into_owned())
    }

    /// Installs `release` on `target` beside the version in use, once per
    /// version whoever noticed it first. Nothing waits for it: a failed
    /// update leaves the version in use, and the next start notices the
    /// newer one again. Its Host admits each operation without waking the
    /// Cloud, and shutdown cancels it.
    fn upgrade_cli(&self, target: CliTarget, release: Release) {
        if !self
            .claude_cli()
            .upgrading
            .borrow_mut()
            .insert(release.version.clone())
        {
            return;
        }
        let shard = self.this();
        self.tasks().spawn_local(async move {
            let upgraded = tokio::select! {
                upgraded = ensure(shard.services(), &target, &release) => Some(upgraded),
                () = shard.closed() => None,
            };
            if let Some(Err(error)) = upgraded {
                tracing::warn!(user = %shard.user(), "the Cloud's Claude Code was not updated: {error}");
            }
            shard.claude_cli().upgrading.borrow_mut().remove(&release.version);
        });
    }

    /// Installs the CLI on the user's Cloud for account work of `entry`, as
    /// a task of the shard, and records the outcome for the settings page.
    /// Shutdown cancels it.
    pub(crate) fn install_cli(&self, entry: ProviderId) {
        let shard = self.this();
        self.tasks().spawn_local(async move {
            let installed = tokio::select! {
                installed = shard.install_cli_now(&entry) => installed,
                () = shard.closed() => return,
            };
            let outcome = match installed {
                Ok(path) => CliInstall::Installed { path },
                Err(message) => CliInstall::Failed { message },
            };
            shard
                .services()
                .cli_installs
                .finish(shard.user(), &entry, outcome);
        });
    }

    async fn install_cli_now(&self, entry: &ProviderId) -> Result<String, String> {
        let access = self
            .cloud_shard()
            .machine_access()
            .await
            .map_err(|error| error.to_string())?;
        self.cli_executable_at(&access, &ProcessWork::Account(entry.clone()))
            .await
    }

    /// The user's Cloud with the CLI versions it has, for `entry`'s
    /// settings, while its runner is connected: it wakes nothing, and a
    /// Cloud that does not answer, or not before `cancel`, has no versions.
    pub async fn cli_machines(
        &self,
        entry: &ProviderId,
        cancel: &CancellationToken,
    ) -> Result<Vec<CliMachine>, StorageError> {
        let control = &self.services().control;
        let Some(device) = control.managed_device(self.user().clone()).await? else {
            return Ok(Vec::new());
        };
        let host = self.devices().device_access(&device.id);
        let home = self.devices().home(&device.id);
        let (Some(host), Some(home)) = (host, home) else {
            return Ok(Vec::new());
        };
        let context = provider_context(control, self.user(), entry).await?;
        let target = CliTarget {
            host,
            home,
            context,
        };
        let listed = cancel
            .run_until_cancelled(installed(self.services(), &target))
            .await;
        let versions = match listed {
            Some(Ok(installed)) => Some(
                installed
                    .into_iter()
                    .map(|installed| installed.version)
                    .collect(),
            ),
            Some(Err(_)) | None => None,
        };
        Ok(vec![CliMachine {
            device_id: device.id,
            name: device.name,
            versions,
        }])
    }
}

/// The versions `target` has, newest first.
async fn installed(services: &Services, target: &CliTarget) -> Result<Vec<Installed>, CliError> {
    let status: Status = call(
        services,
        target,
        Operation::Status,
        Bytes::new(),
        Vec::new(),
        None,
    )
    .await?;
    Ok(status.installed)
}

/// Installs `release` on `target` and answers the executable's path.
async fn ensure(
    services: &Services,
    target: &CliTarget,
    release: &Release,
) -> Result<String, CliError> {
    let failed = |reason: String| CliError {
        version: Some(release.version.clone()),
        reason,
    };
    let record = serde_json::to_vec(release).map_err(|error| failed(error.to_string()))?;
    // The runner may download each of the release's executables from its
    // official URL (`claude-code.md` § The package).
    let attached = release
        .platforms
        .values()
        .map(|artifact| AttachedArtifact {
            artifact: PackageArtifact {
                sha256: artifact.sha256.clone(),
                size: artifact.size,
            },
            location: ArtifactLocation::Url(ArtifactUrl {
                url: artifact.url.clone(),
                expires_at: None,
            }),
        })
        .collect();
    let installed: Installed = call(
        services,
        target,
        Operation::Ensure,
        Bytes::from(record),
        attached,
        Some(&release.version),
    )
    .await?;
    Ok(installed.path.to_string_lossy().into_owned())
}

/// Runs `operation` of the package on `target` with `input` as its whole
/// input and `attached` as the artifacts it may install, and decodes the one
/// document it answers. A nonzero exit keeps a failure document; an output
/// with no document is a service that failed.
async fn call<T: DeserializeOwned>(
    services: &Services,
    target: &CliTarget,
    operation: Operation,
    input: Bytes,
    attached: Vec<AttachedArtifact>,
    version: Option<&str>,
) -> Result<T, CliError> {
    let failed = |reason: String| CliError {
        version: version.map(str::to_owned),
        reason,
    };
    let served = Operation::ALL.map(Operation::name);
    let Some(package) = services
        .native
        .package(PACKAGE)
        .filter(|_| services.native.serves(PACKAGE, &served))
    else {
        return Err(failed(
            "this deployment does not carry the demi.claude-code package, which installs it".into(),
        ));
    };
    let request = ServiceRequest {
        context: target.context.clone(),
        package: package.clone(),
        operation: operation.name().to_owned(),
        args: None,
        json: None,
        cwd: target.home.clone(),
        resolver: services.native.resolver(&services.public_url),
        attached,
    };
    let (output, exit) = match target
        .host
        .call_service(request, input, MAX_ANSWER_BYTES)
        .await
    {
        Ok(output) => (output, None),
        Err(ServiceCallError::Exited {
            exit_code,
            stderr,
            stdout,
        }) => (stdout, Some((exit_code, stderr))),
        Err(error) => return Err(failed(format!("the installer failed: {error}"))),
    };
    if output.iter().all(u8::is_ascii_whitespace) {
        let reason = match exit {
            Some((code, stderr)) => format!("the installer exited with {code}: {}", stderr.trim()),
            None => "the installer gave no answer".into(),
        };
        return Err(failed(reason));
    }
    match (serde_json::from_slice::<Reply<T>>(&output), exit) {
        (Ok(Reply::Failed(failure)), _) => Err(failed(failure.message)),
        (Ok(Reply::Done(answer)), None) => Ok(answer),
        (Ok(Reply::Done(_)), Some((code, _))) => Err(failed(format!(
            "the installer answered and exited with {code}"
        ))),
        (Err(error), _) => Err(failed(format!(
            "the installer's answer cannot be read: {error}"
        ))),
    }
}

/// The outcome of each install that no conversation asked for, by user and
/// entry, kept while the backend runs. A `std` mutex: no section awaits.
#[derive(Default)]
pub struct CliInstalls {
    states: Mutex<HashMap<(UserId, ProviderId), CliInstall>>,
}

impl CliInstalls {
    fn lock(&self) -> MutexGuard<'_, HashMap<(UserId, ProviderId), CliInstall>> {
        // A section only reads or replaces states.
        self.states.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The last install of `entry`'s CLI on `user`'s Cloud.
    pub fn state(&self, user: &UserId, entry: &ProviderId) -> Option<CliInstall> {
        self.lock().get(&(user.clone(), entry.clone())).cloned()
    }

    /// Records an install as started; false when one is under way.
    fn begin(&self, user: &UserId, entry: &ProviderId) -> bool {
        let mut states = self.lock();
        let key = (user.clone(), entry.clone());
        if matches!(states.get(&key), Some(CliInstall::Installing {})) {
            return false;
        }
        states.insert(key, CliInstall::Installing {});
        true
    }

    /// Records how an install ended, unless its entry was deleted meanwhile.
    fn finish(&self, user: &UserId, entry: &ProviderId, outcome: CliInstall) {
        if let Some(state) = self.lock().get_mut(&(user.clone(), entry.clone())) {
            *state = outcome;
        }
    }

    /// Forgets the installs of a deleted entry.
    pub fn forget(&self, entry: &ProviderId) {
        self.lock().retain(|(_, installed), _| installed != entry);
    }
}

/// Starts the install of `entry`'s CLI on `user`'s Cloud unless one is
/// under way, and answers its state. Nobody waits for it.
pub async fn start_install(
    services: &Services,
    shards: &Shards,
    user: &UserId,
    entry: &ProviderId,
) -> CliInstall {
    if !services.cli_installs.begin(user, entry) {
        return CliInstall::Installing {};
    }
    let id = entry.clone();
    let queued = shards
        .of(user)
        .call(move |shard, _| async move { shard.install_cli(id) })
        .await;
    match queued {
        Ok(()) => CliInstall::Installing {},
        Err(error) => {
            let failed = CliInstall::Failed {
                message: error.to_string(),
            };
            services.cli_installs.finish(user, entry, failed.clone());
            failed
        }
    }
}

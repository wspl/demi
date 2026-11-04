//! The commands an integration runs on its Clouds outside any conversation:
//! as the Cloud's init script runs, as `demi` through the Cloud's own access,
//! where `sudo` works, in a context that belongs to no conversation.
//!
//! ```text
//!  POST .../commands  -> id at once -> wake, init script, start -> running
//!  GET  .../commands/:id?after=n    -> its output from byte n, and `next`
//!  ended                            -> row and output stored -> `command` event
//! ```
//!
//! A command holds its Cloud's admission while it runs, which keeps the Cloud
//! awake, and has no time limit: the program stops what it no longer needs.
//! A reset or a deletion of the Cloud stops its commands first, since each
//! waits for the Cloud's admissions to end.
//!
//! The shard holds a command while it runs; its row in the control database
//! (`cloud_commands`) outlives it, with its whole output as a blob, as an
//! agent's command's row does, until its Cloud goes. The row names the
//! command's job before the job is sent, so a command outlives the backend
//! as an agent's does: the Cloud's runner keeps the job, and when it
//! connects to a backend that started again, that backend takes the command
//! up and follows it to its end as if nothing had happened. Until then it
//! reads as running with no output. A command whose start was never sent
//! when the backend stopped reads as failed.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use base64::Engine as _;
use demi_host_interface::Host as _;
use bytes::Bytes;
use demi_backend_database::cloud_commands::{CloudCommandEnd, CloudCommandRow};
use demi_backend_database::control::ControlService;
use demi_backend_database::integrations::IntegrationRecord;
use demi_host_interface::{CommandMedium, Ending, Seen, ShellEnvironment, TakenUp, UnsentStart, WholeOutput};
use demi_shared_types::{BlobRef, CommandEnd, CommandId, EditedFile};
use demi_integration_api_protocol as api;
use demi_runner_client::{CommandKeeper, KeptStart, RemoteShellEnvironment, decode_output, encode_output};
use demi_runner_protocol::wire::JobFileChange;
use futures_util::future::LocalBoxFuture;
use demi_web_api_protocol::ids::{DeviceId, IntegrationId};
use tokio_util::sync::CancellationToken;

use demi_backend_host_access::HostShard as _;
use demi_backend_host_access::access::ConversationHost;
use demi_backend_host_access::transfer::HeldHost;

use crate::integrations::IntegrationRefusal;
use crate::scripts::{script_caller, script_environment, script_request};
use crate::shard::Shard;

/// Why a command whose start an earlier run of the backend never sent reads
/// as failed.
const NEVER_SENT: &str = "Demi restarted before the command started";

/// The title of every command's job.
const TITLE: &str = "Integration command";

/// The commands of the shard's integrations that run, by id.
#[derive(Default)]
pub(crate) struct IntegrationCommands {
    commands: RefCell<HashMap<String, Rc<Command>>>,
}

/// One command on a Cloud.
struct Command {
    integration: IntegrationId,
    cloud: DeviceId,
    /// Ends the command, before or after its start.
    stop: CancellationToken,
    /// Cancelled once `run` is [`Run::Ended`].
    ended: CancellationToken,
    run: RefCell<Run>,
}

/// Where a command is.
enum Run {
    /// Its Cloud wakes, runs its init script, or starts it.
    Starting,
    /// It runs as `command` in `environment`.
    Running {
        environment: Rc<RemoteShellEnvironment>,
        command: CommandId,
    },
    /// It ended: with its exit code and its whole output, or why it did not
    /// run to its end with what it wrote, if anything.
    Ended {
        exit_code: Option<i32>,
        failure: Option<String>,
        output: Option<WholeOutput>,
    },
}

impl Shard {
    /// Starts `start` on the integration's Cloud `cloud` and answers its id
    /// at once: the Cloud wakes and runs its init script first, as for a
    /// conversation's work.
    pub async fn start_cloud_command(
        self: &Rc<Self>,
        integration: &IntegrationRecord,
        cloud: &DeviceId,
        start: api::StartCloudCommand,
    ) -> Result<api::CloudCommand, IntegrationRefusal> {
        // From the check to the registration, so a deletion that starts
        // meanwhile finds the command and stops it.
        let lease = self
            .integration_deletions
            .lease(&[integration.id.as_str(), cloud.as_str()])
            .ok_or_else(|| IntegrationRefusal::Refused("the integration or its Cloud is being deleted".to_owned()))?;
        if self.integration_cloud(integration, cloud).await?.is_none() {
            return Err(IntegrationRefusal::NotFound("Cloud"));
        }
        let device = self
            .services()
            .control
            .device(cloud.clone())
            .await?
            .ok_or(IntegrationRefusal::NotFound("Cloud"))?;
        let id = uuid::Uuid::new_v4().simple().to_string();
        self.services().control.start_cloud_command(id.clone(), cloud.clone()).await?;
        let command = Rc::new(Command {
            integration: integration.id.clone(),
            cloud: cloud.clone(),
            stop: CancellationToken::new(),
            ended: CancellationToken::new(),
            run: RefCell::new(Run::Starting),
        });
        self.integration_commands.commands.borrow_mut().insert(id.clone(), command.clone());
        drop(lease);
        let shard = self.clone();
        let running = id.clone();
        self.follow_cloud_command(id.clone(), command.clone(), async move {
            shard.run_cloud_command(&device, &running, &command, start).await
        });
        self.cloud_command(integration, cloud, &id, None).await
    }

    /// Follows the command `id`, which `run` runs, to its end: then its row
    /// is written and the integration hears of it.
    fn follow_cloud_command(self: &Rc<Self>, id: String, command: Rc<Command>, run: impl Future<Output = Run> + 'static) {
        let shard = self.clone();
        self.tasks().spawn_local(async move {
            let ended = tokio::select! {
                ended = run => ended,
                () = shard.closed() => {
                    // The Cloud's runner keeps the job for the backend that
                    // starts again: the environment lets it go before the
                    // close ends the runner's connection, so an end that
                    // arrives meanwhile does not release the job's output.
                    let running = match &*command.run.borrow() {
                        Run::Running { environment, .. } => Some(environment.clone()),
                        _ => None,
                    };
                    if let Some(environment) = running {
                        environment.detach_all().await;
                    }
                    return;
                }
            };
            *command.run.borrow_mut() = ended;
            command.ended.cancel();
            // Its row first, so a read after the event finds it there,
            // unless the Cloud is being deleted and the row went with it.
            shard.keep_cloud_command(&id, &command).await;
            shard.integration_commands.commands.borrow_mut().remove(&id);
            shard.cloud_command_ended(&id, &command);
        });
    }

    /// Takes up the commands of the Cloud `cloud` that an earlier run of the
    /// backend left running, as its runner connected and was told of their
    /// jobs (`crates/runner-client/src/device_jobs.rs`). The rows are read
    /// again here, after the runner's hello, so a command that ended since
    /// the hello read them is not taken up twice. A command whose start that
    /// run never sent, or that this run holds already, is left as it is.
    pub(crate) fn take_up_cloud_commands(self: &Rc<Self>, cloud: &DeviceId) {
        let shard = self.clone();
        let cloud = cloud.clone();
        self.tasks().spawn_local(async move {
            if let Err(error) = shard.take_up_cloud_commands_now(&cloud, None).await {
                tracing::warn!(event = "cloud_command.take_up_failed", host_id = %cloud, error = &error as &dyn std::error::Error, "the Cloud's running commands were not taken up");
            }
        });
    }

    /// Takes up the running commands of `cloud` that no one follows, or only
    /// the one `only`, and answers whether that one is followed now.
    async fn take_up_cloud_commands_now(self: &Rc<Self>, cloud: &DeviceId, only: Option<&str>) -> Result<bool, demi_backend_database::StorageError> {
        if self.is_closing() {
            return Ok(false);
        }
        let Some(integration) = self.services().control.integration_cloud(cloud.clone()).await? else {
            // The Cloud went, and its rows with it.
            return Ok(false);
        };
        // From the read of the rows to their registration, so a deletion that
        // starts meanwhile finds the commands and stops them.
        let Some(_lease) = self.integration_deletions.lease(&[integration.integration.as_str(), cloud.as_str()]) else {
            return Ok(false);
        };
        let Some(device) = self.services().control.device(cloud.clone()).await? else {
            return Ok(false);
        };
        let running = self.services().control.running_cloud_commands(cloud.clone()).await?;
        let mut followed = false;
        for row in running {
            if only.is_some_and(|only| only != row.id) {
                continue;
            }
            if self.integration_commands.commands.borrow().contains_key(&row.id) {
                followed = true;
                continue;
            }
            let Some(job) = row.job.clone() else {
                continue;
            };
            let command = Rc::new(Command {
                integration: integration.integration.clone(),
                cloud: cloud.clone(),
                stop: CancellationToken::new(),
                ended: CancellationToken::new(),
                run: RefCell::new(Run::Starting),
            });
            self.integration_commands.commands.borrow_mut().insert(row.id.clone(), command.clone());
            followed = true;
            let taking = self.clone();
            let device = device.clone();
            self.follow_cloud_command(row.id.clone(), command.clone(), async move {
                taking.take_up_cloud_command(&device, row, job, &command).await
            });
        }
        Ok(followed)
    }

    /// Follows the command of `row`, which an earlier run of the backend
    /// started on `device` as `job`, from where its job is, and answers how
    /// it ended.
    async fn take_up_cloud_command(&self, device: &demi_backend_database::devices::DeviceRecord, row: CloudCommandRow, job: String, command: &Command) -> Run {
        let failed = |failure: String| Run::Ended {
            exit_code: None,
            failure: Some(failure),
            output: None,
        };
        let woke = tokio::select! {
            access = self.cloud_shard().access(device.clone()) => access,
            () = command.stop.cancelled() => return failed("The command was stopped".into()),
        };
        let access = match woke {
            Ok(access) => access,
            Err(error) => return failed(format!("The Cloud did not start: {error}")),
        };
        let context = match demi_backend_runners::command_context::device_context(&self.services().control, self.user(), &device.id).await {
            Ok(context) => context,
            Err(error) => return failed(error.to_string()),
        };
        let environment = Rc::new(script_environment(access.host.clone(), context, Some(self.cloud_command_keeper(&row.id))));
        let Ok(id) = CommandId::try_from(row.id.clone()) else {
            return failed(format!("the command {} has no id", row.id));
        };
        let ran = self.services().clock.now().as_millisecond() - row.started_at.as_millisecond();
        environment.adopt(TakenUp {
            command: id.clone(),
            job,
            caller: script_caller(TITLE),
            title: TITLE.to_owned(),
            script: String::new(),
            service: true,
            running: std::time::Duration::from_millis(u64::try_from(ran).unwrap_or(0)),
            seen: Seen::default(),
            unsent: None,
        });
        *command.run.borrow_mut() = Run::Running {
            environment: environment.clone(),
            command: id.clone(),
        };
        self.follow_job(&environment, &id, command, CancellationToken::new(), access).await
    }

    /// What records the command `id`'s job before it is sent.
    fn cloud_command_keeper(&self, id: &str) -> Rc<dyn CommandKeeper> {
        Rc::new(CloudCommandKeeper {
            control: self.services().control.clone(),
            id: id.to_owned(),
        })
    }

    /// Runs the command `id` on `device` until it ends or is stopped, and
    /// answers how it ended.
    async fn run_cloud_command(&self, device: &demi_backend_database::devices::DeviceRecord, id: &str, command: &Command, start: api::StartCloudCommand) -> Run {
        let failed = |failure: String| Run::Ended {
            exit_code: None,
            failure: Some(failure),
            output: None,
        };
        let woke = tokio::select! {
            access = self.cloud_shard().access(device.clone()) => access,
            () = command.stop.cancelled() => return failed("The command was stopped before it started".into()),
        };
        let access = match woke {
            Ok(access) => access,
            Err(error) => return failed(format!("The Cloud did not start: {error}")),
        };
        // Its work waits for the init script, as a conversation's does.
        tokio::select! {
            () = self.ready_machine(device) => {}
            () = command.stop.cancelled() => return failed("The command was stopped before it started".into()),
        }
        let context = match demi_backend_runners::command_context::device_context(&self.services().control, self.user(), &device.id).await {
            Ok(context) => context,
            Err(error) => return failed(error.to_string()),
        };
        let paths = access.host.identity().paths;
        let cwd = Some(start.cwd.as_deref().map_or_else(|| access.home.clone(), |cwd| paths.from_home(&access.home, cwd)));
        let script = match &start.stdin {
            // Its input goes with it, whatever bytes it holds.
            Some(stdin) => format!(
                "printf %s '{}' | base64 -d | {{\n{}\n}}",
                base64::engine::general_purpose::STANDARD.encode(stdin),
                start.script
            ),
            None => format!("{{\n{}\n}} </dev/null", start.script),
        };
        let environment = Rc::new(script_environment(access.host.clone(), context, Some(self.cloud_command_keeper(id))));
        let cancel = CancellationToken::new();
        let started = environment.start(script_request(script, cwd, TITLE), cancel.clone()).await;
        let id = match started {
            Ok(id) => id,
            Err(error) => return failed(error.to_string()),
        };
        *command.run.borrow_mut() = Run::Running {
            environment: environment.clone(),
            command: id.clone(),
        };
        self.follow_job(&environment, &id, command, cancel, access).await
    }

    /// Waits for the job of `id` in `environment` to end, or `command` to be
    /// stopped, and answers how it ended; `access` keeps the Cloud awake
    /// until then.
    async fn follow_job(
        &self,
        environment: &Rc<RemoteShellEnvironment>,
        id: &CommandId,
        command: &Command,
        cancel: CancellationToken,
        access: impl Sized,
    ) -> Run {
        let ending = tokio::select! {
            ending = environment.ended(id) => ending,
            // A Cloud whose runner is away, as one that stopped or is reset,
            // ends it as lost instead of waiting for a runner that may not
            // return.
            () = command.stop.cancelled() => {
                environment.dispose_all().await;
                Ok(Ending::Aborted)
            }
        };
        let output = environment.status(id).ok().and_then(|status| status.whole.map(|whole| (*whole.output).clone()));
        let run = match ending {
            Ok(Ending::Exited(exit)) => Run::Ended {
                exit_code: Some(exit),
                failure: None,
                output,
            },
            Ok(Ending::Aborted) => Run::Ended {
                exit_code: None,
                failure: Some("The command was stopped".into()),
                output,
            },
            Ok(Ending::Lost(reason)) => Run::Ended {
                exit_code: None,
                failure: Some(reason),
                output,
            },
            Err(error) => Run::Ended {
                exit_code: None,
                failure: Some(error.to_string()),
                output,
            },
        };
        cancel.cancel();
        environment.dispose_all().await;
        drop(access);
        run
    }

    /// Stops the commands on `cloud` and waits until each has ended, as a
    /// reset or a deletion of it needs.
    pub(crate) async fn end_cloud_commands(&self, cloud: &DeviceId) {
        let commands: Vec<Rc<Command>> = self
            .integration_commands
            .commands
            .borrow()
            .values()
            .filter(|command| command.cloud == *cloud)
            .cloned()
            .collect();
        for command in &commands {
            command.stop.cancel();
        }
        for command in &commands {
            command.ended.cancelled().await;
        }
    }

    /// Stores how the command `id` ended, with its output; a row that is not
    /// written leaves the command reading as one an earlier backend held.
    async fn keep_cloud_command(&self, id: &str, command: &Command) {
        let (exit_code, failure, output) = match &*command.run.borrow() {
            Run::Ended { exit_code, failure, output } => (*exit_code, failure.clone(), output.clone()),
            _ => return,
        };
        let stored = match output {
            Some(output) => {
                let missing = output.missing().cloned();
                // Encoding 16 MiB would hold the shard's thread.
                let encoded = tokio::task::spawn_blocking(move || encode_output(&output)).await;
                let put = match encoded {
                    Ok(Ok(encoded)) => self.blobs().put(Bytes::from(encoded)).await.map_err(|error| error.to_string()),
                    Ok(Err(error)) => Err(error.to_string()),
                    Err(error) => Err(error.to_string()),
                };
                match put {
                    Ok(blob) => Some((blob, missing)),
                    Err(error) => {
                        tracing::warn!(event = "cloud_command.output_not_stored", command = %id, %error, "a Cloud command's output was not stored");
                        None
                    }
                }
            }
            None => None,
        };
        let end = CloudCommandEnd {
            ended_at: self.services().clock.now(),
            exit_code,
            failure,
            output: stored,
        };
        if let Err(error) = self.services().control.end_cloud_command(id.to_owned(), end).await {
            tracing::warn!(event = "cloud_command.end_not_recorded", command = %id, %error, "a Cloud command's end was not recorded");
        }
    }

    /// Tells the integration's sockets that the command `id` ended.
    fn cloud_command_ended(&self, id: &str, command: &Command) {
        let (exit_code, failure) = match &*command.run.borrow() {
            Run::Ended { exit_code, failure, .. } => (*exit_code, failure.clone()),
            _ => return,
        };
        self.integration_events.publish(
            &command.integration,
            api::Event::Command {
                cloud: command.cloud.clone(),
                id: id.to_owned(),
                state: state_of(exit_code),
                exit_code,
                failure,
            },
        );
    }

    /// The integration's command `id` on `cloud`, with its output from the
    /// byte `after`: as its Host keeps it while it runs, as its row keeps it
    /// once it ended.
    pub async fn cloud_command(
        &self,
        integration: &IntegrationRecord,
        cloud: &DeviceId,
        id: &str,
        after: Option<u64>,
    ) -> Result<api::CloudCommand, IntegrationRefusal> {
        let after = after.unwrap_or(0);
        let read = |state, exit_code, failure, output: Option<WholeOutput>| {
            let (output, next) = output.map_or_else(|| (String::new(), after), |output| output.text_after(after));
            api::CloudCommand {
                id: id.to_owned(),
                cloud: cloud.clone(),
                state,
                exit_code,
                failure,
                output,
                next,
            }
        };
        let Some(command) = self.live_command(integration, cloud, id) else {
            return self.stored_cloud_command(integration, cloud, id, read).await;
        };
        let running = match &*command.run.borrow() {
            Run::Running { environment, command } => Some((environment.clone(), command.clone())),
            _ => None,
        };
        if let Some((environment, running)) = running {
            // What its Host keeps of it now. One that ended meanwhile reads
            // as it ended; one whose Host is away reads as no new output.
            let kept = match environment.read_output(&running).await {
                Ok(kept) => Some(kept),
                Err(_) => environment
                    .status(&running)
                    .ok()
                    .and_then(|status| status.whole.map(|whole| (*whole.output).clone())),
            };
            return Ok(read(api::CloudCommandState::Running, None, None, kept));
        }
        let command = match &*command.run.borrow() {
            Run::Ended { exit_code, failure, output } => {
                read(state_of(*exit_code), *exit_code, failure.clone(), output.clone())
            }
            _ => read(api::CloudCommandState::Running, None, None, None),
        };
        Ok(command)
    }

    /// The integration's ended command `id` on `cloud` as its row keeps it.
    async fn stored_cloud_command(
        &self,
        integration: &IntegrationRecord,
        cloud: &DeviceId,
        id: &str,
        read: impl FnOnce(api::CloudCommandState, Option<i32>, Option<String>, Option<WholeOutput>) -> api::CloudCommand,
    ) -> Result<api::CloudCommand, IntegrationRefusal> {
        let row = self
            .services()
            .control
            .cloud_command(id.to_owned())
            .await?
            .filter(|row| row.cloud == *cloud)
            .ok_or(IntegrationRefusal::NotFound("command"))?;
        if self.integration_cloud(integration, cloud).await?.is_none() {
            return Err(IntegrationRefusal::NotFound("command"));
        }
        let Some(end) = row.end else {
            // An earlier run of the backend left it: it runs until the
            // Cloud's runner connects and this run takes it up.
            if row.job.is_some() {
                return Ok(read(api::CloudCommandState::Running, None, None, None));
            }
            return Ok(read(api::CloudCommandState::Failed, None, Some(NEVER_SENT.to_owned()), None));
        };
        let output = match end.output {
            None => None,
            Some((blob, missing)) => {
                let bytes = self
                    .blobs()
                    .get(&blob)
                    .await
                    .map_err(|error| IntegrationRefusal::Internal(error.to_string()))?
                    .ok_or_else(|| IntegrationRefusal::Internal(format!("the output of the command {id} is missing")))?;
                let decoded = tokio::task::spawn_blocking(move || decode_output(&bytes, missing))
                    .await
                    .map_err(|error| IntegrationRefusal::Internal(error.to_string()))?
                    .map_err(|error| IntegrationRefusal::Internal(format!("the output of the command {id} does not decode: {error}")))?;
                Some(decoded)
            }
        };
        Ok(read(state_of(end.exit_code), end.exit_code, end.failure, output))
    }

    /// Stops the integration's command `id` on `cloud`, as the web app's stop
    /// stops an agent's command; one that ended stays as it is.
    pub async fn stop_cloud_command(self: &Rc<Self>, integration: &IntegrationRecord, cloud: &DeviceId, id: &str) -> Result<(), IntegrationRefusal> {
        if let Some(command) = self.live_command(integration, cloud, id) {
            command.stop.cancel();
            return Ok(());
        }
        let Some(row) = self
            .services()
            .control
            .cloud_command(id.to_owned())
            .await?
            .filter(|row| row.cloud == *cloud)
        else {
            return Err(IntegrationRefusal::NotFound("command"));
        };
        if self.integration_cloud(integration, cloud).await?.is_none() {
            return Err(IntegrationRefusal::NotFound("command"));
        }
        // One an earlier run of the backend left running, whose Cloud's
        // runner has not connected since: it is taken up, which wakes the
        // Cloud, and stopped there.
        if row.end.is_none() && row.job.is_some() {
            self.take_up_cloud_commands_now(cloud, Some(id)).await?;
            if let Some(command) = self.live_command(integration, cloud, id) {
                command.stop.cancel();
            }
        }
        Ok(())
    }

    /// The integration's Cloud `cloud` for a file transfer or operation,
    /// awake and kept so while the answer holds it.
    pub async fn integration_cloud_host(&self, integration: &IntegrationRecord, cloud: &DeviceId) -> Result<HeldHost, IntegrationRefusal> {
        if self.integration_cloud(integration, cloud).await?.is_none() {
            return Err(IntegrationRefusal::NotFound("Cloud"));
        }
        let device = self
            .services()
            .control
            .device(cloud.clone())
            .await?
            .ok_or(IntegrationRefusal::NotFound("Cloud"))?;
        let access = self.cloud_shard().access(device).await?;
        let host = ConversationHost {
            host: access.host.clone(),
            root: access.home.clone(),
            home: Some(access.home.clone()),
        };
        Ok(HeldHost {
            host,
            held: Box::new(access),
        })
    }

    /// The integration's command `id` on `cloud` while this shard holds it.
    fn live_command(&self, integration: &IntegrationRecord, cloud: &DeviceId, id: &str) -> Option<Rc<Command>> {
        self.integration_commands
            .commands
            .borrow()
            .get(id)
            .filter(|command| command.integration == integration.id && command.cloud == *cloud)
            .cloned()
    }
}

/// Records a command's job in its row before the job is sent, which is all
/// a backend that starts again needs to take it up; the command's end is
/// written by its follower (`keep_cloud_command`).
struct CloudCommandKeeper {
    control: ControlService,
    /// The command's row.
    id: String,
}

impl CommandKeeper for CloudCommandKeeper {
    fn started<'a>(&'a self, _: &'a CommandId, job: &'a str, _: &'a KeptStart) -> LocalBoxFuture<'a, ()> {
        Box::pin(async move {
            if let Err(error) = self.control.set_cloud_command_job(self.id.clone(), job.to_owned()).await {
                tracing::warn!(event = "cloud_command.job_not_recorded", command = %self.id, error = &error as &dyn std::error::Error, "a Cloud command's job was not recorded");
            }
        })
    }

    /// A Cloud's runner is never away while a command starts on it: its
    /// admission woke it.
    fn waiting<'a>(&'a self, _: &'a CommandId, _: &'a str, _: &'a KeptStart, _: &'a UnsentStart) -> LocalBoxFuture<'a, ()> {
        Box::pin(async {})
    }

    fn retain<'a>(&'a self, _: &'a CommandId, _: &'a [JobFileChange]) -> LocalBoxFuture<'a, Vec<EditedFile>> {
        Box::pin(async { Vec::new() })
    }

    fn attach<'a>(&'a self, _: &'a CommandId, media: &'a [CommandMedium]) -> LocalBoxFuture<'a, Vec<Option<Result<String, String>>>> {
        Box::pin(async move { vec![None; media.len()] })
    }

    fn stored_blob<'a>(&'a self, _: &'a BlobRef) -> LocalBoxFuture<'a, Option<Bytes>> {
        Box::pin(async { None })
    }

    fn looked<'a>(&'a self, _: &'a CommandId, _: Seen) -> LocalBoxFuture<'a, ()> {
        Box::pin(async {})
    }

    fn keep_output<'a>(
        &'a self,
        _: &'a CommandId,
        _: &'a str,
        _: CommandEnd,
        _: &'a WholeOutput,
        _: &'a [CommandMedium],
    ) -> LocalBoxFuture<'a, ()> {
        Box::pin(async {})
    }
}

/// How an ended command's exit code says it ended.
fn state_of(exit_code: Option<i32>) -> api::CloudCommandState {
    match exit_code {
        Some(_) => api::CloudCommandState::Exited,
        None => api::CloudCommandState::Failed,
    }
}

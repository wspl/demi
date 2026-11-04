//! The commands an integration runs on its Clouds outside any conversation:
//! as the Cloud's init script runs, as `demi` through the Cloud's own access,
//! where `sudo` works, in a context that belongs to no conversation.
//!
//! ```text
//!  POST .../commands  -> id at once -> wake, init script, start -> running
//!  GET  .../commands/:id?after=n    -> its output from byte n, and `next`
//!  ended                            -> `command` event; kept for an hour
//! ```
//!
//! A command holds its Cloud's admission while it runs, which keeps the Cloud
//! awake, and has no time limit: the program stops what it no longer needs.
//! A reset or a deletion of the Cloud stops its commands first, since each
//! waits for the Cloud's admissions to end.
//! Its record lives in the shard, which forgets it an hour after it ends, the
//! time a program that reconnects has to read its end, since its output is
//! held in memory; a backend that restarts forgets every one.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use base64::Engine as _;
use demi_host_interface::Host as _;
use demi_backend_database::integrations::IntegrationRecord;
use demi_host_interface::{Ending, ShellEnvironment, WholeOutput};
use demi_shared_types::CommandId;
use demi_integration_api_protocol as api;
use demi_runner_client::RemoteShellEnvironment;
use demi_web_api_protocol::ids::{DeviceId, IntegrationId};
use tokio_util::sync::CancellationToken;

use demi_backend_host_access::access::ConversationHost;
use demi_backend_host_access::transfer::HeldHost;

use crate::integrations::IntegrationRefusal;
use crate::scripts::{script_environment, script_request};
use crate::shard::Shard;

/// How long an ended command's record stays: the hour a program that
/// reconnects has to read its end; its output is held in memory until then.
const KEPT_ENDED: Duration = Duration::from_secs(60 * 60);

/// The commands of the shard's integrations, by id.
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
        self.tasks().spawn_local(async move {
            let ended = tokio::select! {
                ended = shard.run_cloud_command(&device, &command, start) => ended,
                // The shard closes: the runner ends what it ran with it.
                () = shard.closed() => return,
            };
            *command.run.borrow_mut() = ended;
            command.ended.cancel();
            shard.cloud_command_ended(&running, &command);
            tokio::select! {
                () = tokio::time::sleep(KEPT_ENDED) => {}
                () = shard.closed() => {}
            }
            shard.integration_commands.commands.borrow_mut().remove(&running);
        });
        self.cloud_command(integration, cloud, &id, None).await
    }

    /// Runs the command on `device` until it ends or is stopped, and answers
    /// how it ended.
    async fn run_cloud_command(&self, device: &demi_backend_database::devices::DeviceRecord, command: &Command, start: api::StartCloudCommand) -> Run {
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
        let environment = Rc::new(script_environment(access.host.clone(), context));
        let cancel = CancellationToken::new();
        let started = environment.start(script_request(script, cwd, "Integration command"), cancel.clone()).await;
        let id = match started {
            Ok(id) => id,
            Err(error) => return failed(error.to_string()),
        };
        *command.run.borrow_mut() = Run::Running {
            environment: environment.clone(),
            command: id.clone(),
        };
        let ending = tokio::select! {
            ending = environment.ended(&id) => ending,
            // A Cloud whose runner is away, as one that stopped or is reset,
            // ends it as lost instead of waiting for a runner that may not
            // return.
            () = command.stop.cancelled() => {
                environment.dispose_all().await;
                Ok(Ending::Aborted)
            }
        };
        let output = environment.status(&id).ok().and_then(|status| status.whole.map(|whole| (*whole.output).clone()));
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
    /// byte `after`.
    pub async fn cloud_command(
        &self,
        integration: &IntegrationRecord,
        cloud: &DeviceId,
        id: &str,
        after: Option<u64>,
    ) -> Result<api::CloudCommand, IntegrationRefusal> {
        let command = self.integration_command(integration, cloud, id)?;
        let running = match &*command.run.borrow() {
            Run::Running { environment, command } => Some((environment.clone(), command.clone())),
            _ => None,
        };
        let (exit_code, failure, output) = match running {
            Some((environment, running)) => {
                // What its Host keeps of it now. One that ended meanwhile reads
                // as it ended; one whose Host is away reads as no new output.
                let kept = match environment.read_output(&running).await {
                    Ok(kept) => Some(kept),
                    Err(_) => environment
                        .status(&running)
                        .ok()
                        .and_then(|status| status.whole.map(|whole| (*whole.output).clone())),
                };
                (None, None, kept)
            }
            None => match &*command.run.borrow() {
                Run::Ended { exit_code, failure, output } => (*exit_code, failure.clone(), output.clone()),
                _ => (None, None, None),
            },
        };
        let ended = matches!(&*command.run.borrow(), Run::Ended { .. });
        let after = after.unwrap_or(0);
        let (output, next) = output.map_or_else(|| (String::new(), after), |output| output.text_after(after));
        Ok(api::CloudCommand {
            id: id.to_owned(),
            cloud: cloud.clone(),
            state: if ended { state_of(exit_code) } else { api::CloudCommandState::Running },
            exit_code,
            failure,
            output,
            next,
        })
    }

    /// Stops the integration's command `id` on `cloud`, as the web app's stop
    /// stops an agent's command; one that ended stays as it is.
    pub fn stop_cloud_command(&self, integration: &IntegrationRecord, cloud: &DeviceId, id: &str) -> Result<(), IntegrationRefusal> {
        self.integration_command(integration, cloud, id)?.stop.cancel();
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

    fn integration_command(&self, integration: &IntegrationRecord, cloud: &DeviceId, id: &str) -> Result<Rc<Command>, IntegrationRefusal> {
        self.integration_commands
            .commands
            .borrow()
            .get(id)
            .filter(|command| command.integration == integration.id && command.cloud == *cloud)
            .cloned()
            .ok_or(IntegrationRefusal::NotFound("command"))
    }
}

/// How an ended command's exit code says it ended.
fn state_of(exit_code: Option<i32>) -> api::CloudCommandState {
    match exit_code {
        Some(_) => api::CloudCommandState::Exited,
        None => api::CloudCommandState::Failed,
    }
}

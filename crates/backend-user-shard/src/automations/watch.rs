//! Change watches.
//!
//! Every interval, the watch's command runs on the target's device in the
//! target's directory, as the agent's shell would run it:
//!
//! - through the conversation's host access for an existing conversation;
//! - for a new one, each run through device access on a paired device or
//!   machine access on the Cloud, which wakes it
//!   (`crates/backend-host-access/src/lib.rs`).
//!
//! The SHA-256 of its whole standard output and its exit status are compared
//! with the last ones, and its output's last 16 KB kept for the message. The
//! first run only records them, and a change fires a run.
//!
//! - A command that runs over 60 seconds is stopped and fails the check.
//! - A check a pause, an edit of the trigger or a deletion overtook is stopped,
//!   and records nothing.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use demi_backend_database::automations::{RunCause, StoredAutomation, WatchOutput};
use demi_backend_page_sync::Part;
use demi_backend_remote_host::{ContextSource, EnvironmentOptions, RemoteHost, RemoteShellEnvironment};
use demi_command_protocol::{CommandCaller, CommandContext, Viewable};
use demi_host_interface::{
    CommandRecord, Ending, ExecRequest, HostError, JobCaller, Numbers, OutputRecord, PageFeed, ShellEnvironment,
};
use demi_shared_types::{NodeId, Sequence, StreamKind, Timestamp};
use demi_web_api_protocol::automations::{AutomationTarget, RunNote, RunState, Trigger};
use demi_web_api_protocol::conversations::ConversationTarget;
use demi_web_api_protocol::devices::DeviceKind;
use futures_util::future::LocalBoxFuture;
use sha2::{Digest, Sha256};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use super::message::{CUT_BYTES, Output, last_bytes};
use super::runs::{Fired, Payload, Unrunnable};
use crate::shard::Shard;

/// How long a watch's command may run.
pub(crate) const COMMAND_TIME: Duration = Duration::from_secs(60);

/// How a check ended.
enum Checked {
    /// Its output, with the digest of the whole of its standard output.
    Output(Output, Vec<u8>),
    Unrunnable(Unrunnable),
    TimedOut,
    Failed(String),
}

/// A watch's jobs have no page that follows them.
struct NoPage(watch::Sender<bool>);

impl PageFeed for NoPage {
    fn changed(&self, _: &Rc<RefCell<CommandRecord>>) {}

    fn watching(&self) -> watch::Receiver<bool> {
        self.0.subscribe()
    }
}

/// A watch's command numbers, which no one reads.
#[derive(Default)]
struct Count(Cell<u64>);

impl Numbers for Count {
    fn next(&self, _: Sequence) -> LocalBoxFuture<'_, Result<u64, HostError>> {
        self.0.set(self.0.get() + 1);
        let next = self.0.get();
        Box::pin(async move { Ok(next) })
    }
}

/// Runs `script` in `cwd` on `host` as the agent's shell would, and answers
/// its output, or why there is none.
async fn run_command(
    host: RemoteHost,
    cwd: Option<String>,
    context: CommandContext,
    script: String,
    stop: &CancellationToken,
) -> Checked {
    let source: ContextSource = Rc::new(move || {
        let context = context.clone();
        Box::pin(async move { Ok(context) })
    });
    let environment = RemoteShellEnvironment::new(EnvironmentOptions::new(
        host,
        source,
        Rc::new(NoPage(watch::Sender::new(false))),
        Rc::new(Count::default()),
    ));
    let request = ExecRequest {
        script,
        cwd,
        caller: JobCaller {
            node: NodeId::try_from("automation").expect("a name is not empty"),
            tool_use_id: "watch".into(),
            turn: None,
        },
        title: "Change watch".into(),
        interval_ms: None,
        viewable: Viewable {
            model: "none".into(),
            media_types: Some(Vec::new()),
        },
    };
    let cancel = CancellationToken::new();
    let command = match environment.start(request, cancel.clone()).await {
        Ok(command) => command,
        Err(error) => return Checked::Failed(error.to_string()),
    };
    let ended = tokio::select! {
        ended = tokio::time::timeout(COMMAND_TIME, environment.ended(&command)) => Some(ended),
        () = stop.cancelled() => None,
    };
    let checked = match ended {
        // Dropped by a pause, an edit or a deletion: what it gave is not read.
        None => Checked::Failed("the check was dropped".into()),
        Some(Err(_)) => Checked::TimedOut,
        Some(Ok(Err(error))) => Checked::Failed(error.to_string()),
        Some(Ok(Ok(Ending::Aborted))) => Checked::Failed("the command was stopped".into()),
        Some(Ok(Ok(Ending::Lost(reason)))) => Checked::Failed(reason),
        Some(Ok(Ok(Ending::Exited(exit)))) => match environment.status(&command) {
            Ok(status) => {
                let mut stdout = Vec::new();
                let mut digest = Sha256::new();
                if let Some(whole) = status.whole {
                    for record in whole.output.records() {
                        match record {
                            OutputRecord::Output(StreamKind::Stdout, bytes) => {
                                digest.update(bytes);
                                stdout.extend_from_slice(bytes);
                                if stdout.len() > 2 * CUT_BYTES {
                                    stdout = last_bytes(&stdout, CUT_BYTES);
                                }
                            }
                            // Past the 16 MB the Host keeps, the middle is
                            // left out: its length stands for it.
                            OutputRecord::LeftOut(bytes) => digest.update(format!("\0left out {bytes}\0")),
                            OutputRecord::Output(..) => {}
                        }
                    }
                }
                Checked::Output(
                    Output {
                        stdout: last_bytes(&stdout, CUT_BYTES),
                        exit,
                    },
                    digest.finalize().to_vec(),
                )
            }
            Err(error) => Checked::Failed(error.to_string()),
        },
    };
    // A command over its time, or of a dropped check, is stopped, and every
    // one let go of.
    cancel.cancel();
    environment.dispose_all().await;
    checked
}

impl Shard {
    /// Whether the watch's check runs now, which the scheduler does not
    /// start again.
    pub(crate) fn automation_checking(&self, id: &demi_shared_types::AutomationId) -> bool {
        self.automations()
            .pending
            .borrow()
            .get(id)
            .is_some_and(|pending| pending.check.is_some())
    }

    /// Runs the watch's check in a task of its own, which a pause, an edit
    /// of the trigger or a deletion stops; the scheduler starts it again
    /// only after it ends.
    pub(crate) fn start_watch(&self, automation: StoredAutomation) {
        let mut pending = self.automations().pending.borrow_mut();
        let entry = pending.entry(automation.id.clone()).or_default();
        if entry.check.is_some() {
            return;
        }
        let generation = entry.generation;
        let stop = CancellationToken::new();
        entry.check = Some(stop.clone().drop_guard());
        drop(pending);
        let shard = self.this();
        self.tasks().spawn_local(async move {
            shard.check_watch(&automation, generation, &stop).await;
            let now = shard.services().control.now();
            shard.automations().tried.borrow_mut().insert(automation.id.clone(), now);
            let done = {
                let mut pending = shard.automations().pending.borrow_mut();
                pending
                    .get_mut(&automation.id)
                    .filter(|entry| entry.generation == generation)
                    .and_then(|entry| entry.check.take())
            };
            // Its own guard: the check is over, nothing to stop.
            if let Some(done) = done {
                done.disarm();
            }
            #[cfg(feature = "testing")]
            shard
                .services()
                .watches
                .pass(crate::holds::WatchStep::Ended)
                .await;
            shard.rearm_automations();
        });
    }

    async fn check_watch(&self, automation: &StoredAutomation, generation: u64, stop: &CancellationToken) {
        let Trigger::Change { command, .. } = &automation.trigger else {
            return;
        };
        let control = &self.services().control;
        let checked = self.run_watch_command(automation, command.clone(), stop).await;
        let now = control.now();
        if !self.automation_current(&automation.id, generation) {
            return;
        }
        let note = match checked {
            Checked::Output(output, digest) => {
                if let Err(error) = self.compare(automation, output, digest, generation, now).await {
                    tracing::warn!(automation = %automation.id, "a watch's output was not recorded: {error}");
                }
                return;
            }
            Checked::Unrunnable(Unrunnable::Offline(device)) => (RunState::Skipped, RunNote::Offline { device }),
            // `place_after_return` waited for it already.
            Checked::Unrunnable(Unrunnable::Returning { device, .. }) => (
                RunState::Skipped,
                RunNote::Offline { device: device.to_string() },
            ),
            Checked::Unrunnable(Unrunnable::Gone(reason)) => {
                if let AutomationTarget::Existing { conversation_id } = &automation.target {
                    self.automation_target_gone(conversation_id, reason).await;
                }
                (RunState::Skipped, RunNote::TargetGone)
            }
            Checked::TimedOut => (RunState::Failed, RunNote::CommandTimedOut),
            Checked::Failed(message) => (RunState::Failed, RunNote::CommandFailed { message }),
        };
        let mut run = super::runs::skipped(automation, RunCause::Change, now, note.1);
        run.state = note.0;
        match control.insert_automation_run(run).await {
            Ok(()) => self.mark(Part::Automations),
            Err(error) => tracing::warn!(automation = %automation.id, "a watch's run was not recorded: {error}"),
        }
    }

    /// Compares a check's output with the last one recorded: the first only
    /// records it, the same one moves its time, and a changed one fires a
    /// run, unless the day's limit was reached, which keeps the old one so
    /// the change fires the next day.
    async fn compare(
        &self,
        automation: &StoredAutomation,
        output: Output,
        digest: Vec<u8>,
        generation: u64,
        now: Timestamp,
    ) -> Result<(), demi_backend_database::StorageError> {
        let control = &self.services().control;
        let record = WatchOutput {
            stdout: output.stdout.clone(),
            digest,
            exit: output.exit,
            checked_at: now,
        };
        let Some(before) = &automation.watch else {
            return control.set_watch_output(automation.id.clone(), Some(record)).await;
        };
        if before.digest == record.digest && before.exit == output.exit {
            return control.set_watch_output(automation.id.clone(), Some(record)).await;
        }
        if self.limit_reached(automation, now).await?.is_some() {
            return self.count_held_back(automation, 1).await;
        }
        // Read anew after the wait: a change since drops this check.
        if !self.automation_current(&automation.id, generation) {
            return Ok(());
        }
        control.set_watch_output(automation.id.clone(), Some(record)).await?;
        let fired = Fired {
            cause: RunCause::Change,
            at: now,
            payload: Payload::Change {
                before: Output {
                    stdout: before.stdout.clone(),
                    exit: before.exit,
                },
                now: output,
            },
        };
        self.fire(automation, fired).await;
        Ok(())
    }

    /// Runs the watch's command where its target is, or answers why it
    /// cannot.
    async fn run_watch_command(&self, automation: &StoredAutomation, script: String, stop: &CancellationToken) -> Checked {
        let control = &self.services().control;
        if let Err(unrunnable) = self.place_after_return(automation).await {
            return Checked::Unrunnable(unrunnable);
        }
        let failed = |error: &dyn std::fmt::Display| Checked::Failed(error.to_string());
        match &automation.target {
            AutomationTarget::Existing { conversation_id } => {
                let context = match demi_backend_runners::command_context::command_context(
                    control,
                    self.user(),
                    conversation_id,
                    CommandCaller::User {},
                )
                .await
                {
                    Ok(context) => context,
                    Err(error) => return failed(&error),
                };
                let cancel = CancellationToken::new();
                self.host_shard()
                    .with_host(conversation_id, None, &cancel, async |host| {
                        run_command(host.host.clone(), None, context, script, stop).await
                    })
                    .await
                    .unwrap_or_else(|error| failed(&error))
            }
            AutomationTarget::New { target } => {
                let context = match demi_backend_runners::command_context::automation_context(
                    control,
                    self.user(),
                    automation.id.as_str(),
                )
                .await
                {
                    Ok(context) => context,
                    Err(error) => return failed(&error),
                };
                let (device, path) = match target {
                    ConversationTarget::Cloud { path } => (None, path.clone()),
                    ConversationTarget::Device { device_id, path } => (Some(device_id.clone()), Some(path.clone())),
                    ConversationTarget::Workspace { workspace_id } => {
                        match control.workspace(workspace_id.clone()).await {
                            Ok(Some(workspace)) => (Some(workspace.device), Some(workspace.path)),
                            Ok(None) => return Checked::Failed("its project was deleted".into()),
                            Err(error) => return failed(&error),
                        }
                    }
                };
                let managed = match &device {
                    None => true,
                    Some(device) => matches!(
                        control.device(device.clone()).await,
                        Ok(Some(record)) if record.kind == DeviceKind::Managed
                    ),
                };
                if managed {
                    match self.cloud_shard().machine_access().await {
                        Ok(access) => {
                            let cwd = path.unwrap_or_else(|| access.home.clone());
                            run_command(access.host.clone(), Some(cwd), context, script, stop).await
                        }
                        Err(error) => failed(&error),
                    }
                } else {
                    let device = device.expect("a paired device's target names it");
                    match self.devices().device_access(&device) {
                        Some(host) => run_command(host, path, context, script, stop).await,
                        None => Checked::Unrunnable(Unrunnable::Offline(
                            self.offline_device(&device).await.unwrap_or_else(|| device.to_string()),
                        )),
                    }
                }
            }
        }
    }
}

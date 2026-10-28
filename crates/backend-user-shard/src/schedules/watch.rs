//! Change watches.
//!
//! Every interval, the watch's command runs on the conversation's Host in its
//! directory, as the agent's shell would run it, through the conversation's host
//! access.
//!
//! The SHA-256 of its whole standard output and its exit status are compared
//! with the last ones, and its output's last 16 KB kept for the message. The
//! first run only records them, and a change fires a run.
//!
//! - A command that runs over 60 seconds is stopped and fails the check.
//! - A check a pause, an edit of the trigger or a deletion overtook is stopped,
//!   and records nothing.

use std::time::Duration;

use demi_backend_database::schedules::{RunCause, StoredSchedule, WatchOutput};
use demi_backend_page_sync::Part;
use demi_runner_client::RemoteHost;
use demi_command_protocol::{CommandCaller, CommandContext};
use demi_host_interface::OutputRecord;
use demi_shared_types::{StreamKind, Timestamp};
use demi_web_api_protocol::schedules::{RunNote, RunState, Trigger};
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

use super::message::{CUT_BYTES, Output, last_bytes};
use super::runs::{Fired, Payload, Unrunnable};
use crate::scripts::{ScriptEnd, run_script};
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

/// Runs `script` on `host` as the agent's shell would, at most
/// [`COMMAND_TIME`], and answers its output, or why there is none.
async fn run_command(
    host: RemoteHost,
    context: CommandContext,
    script: String,
    stop: &CancellationToken,
) -> Checked {
    let ended = run_script(host, None, context, script, "Change watch", Some(COMMAND_TIME), stop, |exit, whole| {
        let mut stdout = Vec::new();
        let mut digest = Sha256::new();
        if let Some(whole) = whole {
            for record in whole.records() {
                match record {
                    OutputRecord::Output(StreamKind::Stdout, bytes) => {
                        digest.update(bytes);
                        stdout.extend_from_slice(bytes);
                        if stdout.len() > 2 * CUT_BYTES {
                            stdout = last_bytes(&stdout, CUT_BYTES);
                        }
                    }
                    // Past the 16 MB the Host keeps, the middle is left
                    // out: its length stands for it.
                    OutputRecord::LeftOut(bytes) => digest.update(format!("\0left out {bytes}\0")),
                    OutputRecord::Output(..) => {}
                }
            }
        }
        (
            Output {
                stdout: last_bytes(&stdout, CUT_BYTES),
                exit,
            },
            digest.finalize().to_vec(),
        )
    })
    .await;
    match ended {
        ScriptEnd::Read((output, digest)) => Checked::Output(output, digest),
        ScriptEnd::TimedOut => Checked::TimedOut,
        // Dropped by a pause, an edit or a deletion: what it gave is not read.
        ScriptEnd::Dropped => Checked::Failed("the check was dropped".into()),
        ScriptEnd::Failed(reason) => Checked::Failed(reason),
    }
}


impl Shard {
    /// Whether the watch's check runs now, which the scheduler does not
    /// start again.
    pub(crate) fn schedule_checking(&self, id: &demi_shared_types::ScheduleId) -> bool {
        self.schedules()
            .pending
            .borrow()
            .get(id)
            .is_some_and(|pending| pending.check.is_some())
    }

    /// Runs the watch's check in a task of its own, which a pause, an edit
    /// of the trigger or a deletion stops; the scheduler starts it again
    /// only after it ends.
    pub(crate) fn start_watch(&self, schedule: StoredSchedule) {
        let mut pending = self.schedules().pending.borrow_mut();
        let entry = pending.entry(schedule.id.clone()).or_default();
        if entry.check.is_some() {
            return;
        }
        let generation = entry.generation;
        let stop = CancellationToken::new();
        entry.check = Some(stop.clone().drop_guard());
        drop(pending);
        let shard = self.this();
        self.tasks().spawn_local(async move {
            shard.check_watch(&schedule, generation, &stop).await;
            let now = shard.services().control.now();
            shard.schedules().tried.borrow_mut().insert(schedule.id.clone(), now);
            let done = {
                let mut pending = shard.schedules().pending.borrow_mut();
                pending
                    .get_mut(&schedule.id)
                    .filter(|entry| entry.generation == generation)
                    .and_then(|entry| entry.check.take())
            };
            // Its own guard: the check is over, nothing to stop.
            if let Some(done) = done {
                done.disarm();
            }
            shard.rearm_schedules();
        });
    }

    async fn check_watch(&self, schedule: &StoredSchedule, generation: u64, stop: &CancellationToken) {
        let Trigger::Change { command, .. } = &schedule.trigger else {
            return;
        };
        let control = &self.services().control;
        let checked = self.run_watch_command(schedule, command.clone(), stop).await;
        let now = control.now();
        if !self.schedule_current(&schedule.id, generation) {
            return;
        }
        let note = match checked {
            Checked::Output(output, digest) => {
                if let Err(error) = self.compare(schedule, output, digest, generation, now).await {
                    tracing::warn!(event = "schedule_watch.record_failed", schedule = %schedule.id, what = "output", %error, "a watch's output was not recorded");
                }
                return;
            }
            Checked::Unrunnable(Unrunnable::Offline(device)) => (RunState::Skipped, RunNote::Offline { device }),
            // `runnable_after_return` waited for it already.
            Checked::Unrunnable(Unrunnable::Returning { device, .. }) => (
                RunState::Skipped,
                RunNote::Offline { device: device.to_string() },
            ),
            // Its conversation is archived or gone, which keeps it from waking or deletes
            // the schedule: nothing to record.
            Checked::Unrunnable(Unrunnable::Gone) => return,
            Checked::TimedOut => (RunState::Failed, RunNote::CommandTimedOut),
            Checked::Failed(message) => (RunState::Failed, RunNote::CommandFailed { message }),
        };
        let mut run = super::runs::skipped(schedule, RunCause::Change, now, note.1);
        run.state = note.0;
        match control.insert_schedule_run(run).await {
            Ok(()) => self.mark(Part::Metadata(schedule.conversation.clone())),
            Err(error) => tracing::warn!(event = "schedule_watch.record_failed", schedule = %schedule.id, what = "run", %error, "a watch's run was not recorded"),
        }
    }

    /// Compares a check's output with the last one recorded: the first only
    /// records it, the same one moves its time, and a changed one fires a
    /// run, unless the day's limit was reached, which keeps the old one so
    /// the change fires the next day.
    async fn compare(
        &self,
        schedule: &StoredSchedule,
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
        let Some(before) = &schedule.watch else {
            return control.set_watch_output(schedule.id.clone(), Some(record)).await;
        };
        if before.digest == record.digest && before.exit == output.exit {
            return control.set_watch_output(schedule.id.clone(), Some(record)).await;
        }
        if self.limit_reached(schedule, now).await?.is_some() {
            return self.count_held_back(schedule, 1).await;
        }
        // Read anew after the wait: a change since drops this check.
        if !self.schedule_current(&schedule.id, generation) {
            return Ok(());
        }
        control.set_watch_output(schedule.id.clone(), Some(record)).await?;
        let fired = Fired {
            cause: RunCause::Change,
            task: None,
            at: now,
            payload: Payload::Change {
                before: Output {
                    stdout: before.stdout.clone(),
                    exit: before.exit,
                },
                now: output,
            },
        };
        self.fire(schedule, fired).await;
        Ok(())
    }

    /// Runs the watch's command on the conversation's Host as the agent's
    /// shell would, or answers why it cannot.
    async fn run_watch_command(&self, schedule: &StoredSchedule, script: String, stop: &CancellationToken) -> Checked {
        let control = &self.services().control;
        if let Err(unrunnable) = self.runnable_after_return(schedule).await {
            return Checked::Unrunnable(unrunnable);
        }
        let failed = |error: &dyn std::fmt::Display| Checked::Failed(error.to_string());
        let context = match demi_backend_runners::command_context::command_context(
            control,
            self.user(),
            &schedule.conversation,
            CommandCaller::User {},
        )
        .await
        {
            Ok(context) => context,
            Err(error) => return failed(&error),
        };
        let cancel = CancellationToken::new();
        self.host_shard()
            .with_host(&schedule.conversation, None, &cancel, async |host| {
                run_command(host.host.clone(), context, script, stop).await
            })
            .await
            .unwrap_or_else(|error| failed(&error))
    }
}

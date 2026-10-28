//! An integration's init script on its Clouds.
//!
//! A Cloud of an integration runs the integration's script once its system
//! is new: at its first use, and after every reset. A conversation's work on
//! the Cloud waits for the run, once the Cloud admitted it; the run itself
//! takes the Cloud's admission as the backend's own work, so it never waits
//! for itself.
//!
//! ```text
//!            first use, a script             its end
//!   pending ---------------------> running ----------> done | failed
//!      |
//!      |     first use, no script
//!      +------------------------> unscripted
//!
//!   a reset, from any state -----> pending
//! ```
//!
//! A change of the script reaches no Cloud whose system was made already:
//! only a reset gives a Cloud a new system, on which the script it finds
//! then runs.
//!
//! A Cloud whose run failed serves its conversations as one whose run did
//! not: the integration's program reads the run and decides what a failure
//! means to its people. A run has no time limit, since a script that
//! installs packages may take long, and its output is kept as a command's
//! is.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use demi_backend_database::devices::DeviceRecord;
use demi_web_api_protocol::ids::DeviceId;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use crate::scripts::{ScriptEnd, run_script};
use crate::shard::Shard;

/// The runs under way in this backend, by Cloud, with the end the work
/// waiting for each hears. A Cloud's work waits while its run is here,
/// whatever its record says.
#[derive(Default)]
pub(crate) struct InitRuns {
    running: RefCell<HashMap<DeviceId, Rc<Notify>>>,
}

impl Shard {
    /// Once a conversation's work is admitted on `device`: waits until a
    /// Cloud of an integration that has a script has run it, starting the
    /// run when it owes one.
    pub(crate) async fn ready_machine(&self, device: &DeviceRecord) {
        loop {
            let under_way = self.init_runs.running.borrow().get(&device.id).cloned();
            if let Some(ended) = under_way {
                let notified = ended.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                if self.init_runs.running.borrow().contains_key(&device.id) {
                    notified.await;
                }
                continue;
            }
            // From the check above to here nothing awaits: one work of the
            // Cloud's claims its run, and the rest wait for it.
            let ended = Rc::new(Notify::new());
            self.init_runs.running.borrow_mut().insert(device.id.clone(), ended.clone());
            match self.services().control.claim_cloud_init(device.id.clone()).await {
                Ok(Some(script)) => {
                    let shard = self.this();
                    let device = device.clone();
                    self.tasks().spawn_local(async move { shard.run_init(device, script).await });
                }
                claimed => {
                    if let Err(error) = claimed {
                        tracing::warn!(event = "integration_cloud.init_unread", device = %device.id, %error, "a Cloud's init script was not read; its work goes on without it");
                    }
                    self.init_runs.running.borrow_mut().remove(&device.id);
                    ended.notify_waiters();
                    return;
                }
            }
        }
    }

    /// Runs `script` on the Cloud `device` as `demi` in its home, records how
    /// it ended and lets the work waiting for it go, whether or not the
    /// record was written: one that was not leaves the run owed.
    async fn run_init(self: Rc<Self>, device: DeviceRecord, script: String) {
        let ran = tokio::select! {
            ran = self.init_on(&device, script) => ran,
            // The record stays running, which the next start runs again.
            () = self.closed() => return,
        };
        let (exit_code, output) = match ran {
            Ok((exit, output)) => (Some(exit), output),
            Err(reason) => (None, reason),
        };
        if let Err(error) = self.services().control.end_cloud_init(device.id.clone(), exit_code, output).await {
            tracing::warn!(event = "integration_cloud.init_not_recorded", device = %device.id, %error, "an init script's run was not recorded; it runs again at the Cloud's next use");
        }
        if let Some(ended) = self.init_runs.running.borrow_mut().remove(&device.id) {
            ended.notify_waiters();
        }
    }

    /// The exit code and the output of `script` on `device`, or
    /// why it did not run.
    async fn init_on(&self, device: &DeviceRecord, script: String) -> Result<(i32, String), String> {
        let control = &self.services().control;
        let access = self
            .cloud_shard()
            .access(device.clone())
            .await
            .map_err(|error| format!("The Cloud did not start: {error}"))?;
        let context = demi_backend_runners::command_context::device_context(control, self.user(), &device.id)
            .await
            .map_err(|error| error.to_string())?;
        let stop = CancellationToken::new();
        let ended = run_script(access.host.clone(), Some(access.home.clone()), context, script, "Init script", None, &stop, |exit, whole| {
            (exit, whole.map(|whole| whole.text_after(0).0).unwrap_or_default())
        })
            .await;
        match ended {
            ScriptEnd::Read(read) => Ok(read),
            ScriptEnd::TimedOut | ScriptEnd::Dropped => Err("The run was stopped".into()),
            ScriptEnd::Failed(reason) => Err(reason),
        }
    }
}

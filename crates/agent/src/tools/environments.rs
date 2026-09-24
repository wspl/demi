//! A node's shell environments (`runtime.md` § Running shell tools): one per
//! Host the node used, keyed by the Host's key. Concurrent calls for one Host
//! make one environment; a handle belongs to the environment that made it;
//! and the repeat guard of identical `shell_exec` scripts lives beside each
//! environment.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

use demi_core::{CommandId, ShellId, ToolResultContentBlock, ToolView};
use demi_shell::{HostError, HostKey, ShellEnvironment};
use futures_util::future::join_all;
use tokio::{sync::OnceCell, time::Instant};

use crate::session::ToolOutcome;

/// How many identical `shell_exec` calls in a row run.
const MAX_IDENTICAL_EXECS: u32 = 6;
/// How long after the previous exec an identical one still counts as a
/// repeat.
const REPEAT_WINDOW: Duration = Duration::from_secs(60);

/// The environments of one node.
#[derive(Default)]
pub(crate) struct Environments {
    slots: RefCell<Vec<Rc<Slot>>>,
    disposed: Cell<bool>,
}

/// One Host's environment, once made, and its repeat guard.
pub(super) struct Slot {
    key: HostKey,
    environment: OnceCell<Rc<dyn ShellEnvironment>>,
    repeat: RefCell<Option<Repeat>>,
}

/// The last `shell_exec` script and how many times in a row it ran.
struct Repeat {
    script: String,
    count: u32,
    at: Instant,
}

/// A handle a call carries, which must belong to the call's environment.
#[derive(Clone, Copy)]
pub(super) enum Handle<'a> {
    None,
    Shell(&'a ShellId),
    Command(&'a CommandId),
}

impl Environments {
    /// The environment for the Host `key`, made with `create` unless it was
    /// made before; a concurrent call for the same Host waits for the same
    /// creation. The handle `handle` must be this environment's: one another
    /// Host's environment owns is refused, as is one two environments claim.
    pub(super) async fn resolve(
        &self,
        key: HostKey,
        handle: Handle<'_>,
        create: impl Future<Output = Result<Rc<dyn ShellEnvironment>, HostError>>,
    ) -> Result<(Rc<Slot>, Rc<dyn ShellEnvironment>), String> {
        if self.disposed.get() {
            return Err(DISPOSED.to_owned());
        }
        let slot = self.slot(key);
        let environment = slot
            .environment
            .get_or_try_init(|| create)
            .await
            .map_err(|error| error.to_string())?
            .clone();
        if self.disposed.get() {
            // Made while the node was being disposed: it goes too.
            environment.dispose_all().await;
            return Err(DISPOSED.to_owned());
        }
        if let Some(owner) = self.owner(handle)?
            && owner.key != slot.key
        {
            return Err(format!(
                "Shell handle \"{}\" belongs to a different Host",
                handle.id()
            ));
        }
        Ok((slot, environment))
    }

    fn slot(&self, key: HostKey) -> Rc<Slot> {
        let mut slots = self.slots.borrow_mut();
        if let Some(slot) = slots.iter().find(|slot| slot.key == key) {
            return slot.clone();
        }
        let slot = Rc::new(Slot {
            key,
            environment: OnceCell::new(),
            repeat: RefCell::new(None),
        });
        slots.push(slot.clone());
        slot
    }

    /// The slot whose environment owns `handle`, when one does.
    fn owner(&self, handle: Handle<'_>) -> Result<Option<Rc<Slot>>, String> {
        let owns = |environment: &Rc<dyn ShellEnvironment>| match handle {
            Handle::None => false,
            Handle::Shell(shell) => environment.owns_shell(shell),
            Handle::Command(command) => environment.owns_command(command),
        };
        let owners: Vec<Rc<Slot>> = self
            .slots
            .borrow()
            .iter()
            .filter(|slot| slot.environment.get().is_some_and(owns))
            .cloned()
            .collect();
        match owners.as_slice() {
            [] => Ok(None),
            [owner] => Ok(Some(owner.clone())),
            _ => Err(match handle {
                Handle::Shell(shell) => {
                    format!("Shell id \"{shell}\" is not unique in this session")
                }
                Handle::Command(command) => {
                    format!("Command id \"{command}\" is not unique in this session")
                }
                Handle::None => unreachable!("no environment owns no handle"),
            }),
        }
    }

    /// The environment that owns `command`, among those made.
    pub(super) fn owning(&self, command: &CommandId) -> Option<Rc<dyn ShellEnvironment>> {
        self.slots
            .borrow()
            .iter()
            .filter_map(|slot| slot.environment.get())
            .find(|environment| environment.owns_command(command))
            .cloned()
    }

    /// Ends every shell of every environment made so far and forgets them,
    /// so the next call on a Host makes a fresh environment there.
    pub(crate) async fn end_all(&self) {
        let slots: Vec<Rc<Slot>> = self.slots.borrow_mut().drain(..).collect();
        let environments: Vec<Rc<dyn ShellEnvironment>> = slots
            .iter()
            .filter_map(|slot| slot.environment.get().cloned())
            .collect();
        join_all(
            environments
                .iter()
                .map(|environment| environment.dispose_all()),
        )
        .await;
    }

    /// Ends every shell of every environment, and every environment made
    /// from now on.
    pub(crate) async fn dispose(&self) {
        self.disposed.set(true);
        let environments: Vec<Rc<dyn ShellEnvironment>> = self
            .slots
            .borrow()
            .iter()
            .filter_map(|slot| slot.environment.get().cloned())
            .collect();
        join_all(
            environments
                .iter()
                .map(|environment| environment.dispose_all()),
        )
        .await;
    }
}

/// What a call answers once the node is being disposed.
const DISPOSED: &str = "The node's shells are closed";

impl Handle<'_> {
    fn id(&self) -> &str {
        match self {
            Handle::None => "",
            Handle::Shell(shell) => shell.as_str(),
            Handle::Command(command) => command.as_str(),
        }
    }
}

impl Slot {
    /// Counts `script` against the repeat guard: within the window of the
    /// previous exec, the same script runs six times in a row, and the
    /// seventh and later are suppressed; another script starts the count
    /// again.
    pub(super) fn repeated(&self, script: &str) -> Option<ToolOutcome> {
        let now = Instant::now();
        let mut repeat = self.repeat.borrow_mut();
        let count = match repeat.as_ref() {
            Some(previous) if previous.script == script && now - previous.at <= REPEAT_WINDOW => {
                previous.count + 1
            }
            _ => 1,
        };
        *repeat = Some(Repeat {
            script: script.to_owned(),
            count,
            at: now,
        });
        if count <= MAX_IDENTICAL_EXECS {
            return None;
        }
        let text = [
            "Repeated identical shell_exec suppressed.".to_owned(),
            format!("The same script has been run {count} consecutive times in this agent session."),
            "Inspect the previous output, use a different command, or provide the final answer instead of repeating it.".to_owned(),
        ]
        .join("\n");
        Some(ToolOutcome {
            output: vec![ToolResultContentBlock::Text { text }],
            is_error: true,
            view: Some(ToolView::RepeatedShellExec {
                script: script.to_owned(),
                count,
            }),
            effect: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use bytes::Bytes;
    use demi_shell::{CommandStatus, ExecRequest, Reader, ShellError};
    use futures_util::future::{LocalBoxFuture, join};
    use tokio_util::sync::CancellationToken;

    use super::*;

    /// An environment that owns the commands it is given and counts its
    /// disposals; the tools' other operations are not what these tests call.
    #[derive(Default)]
    struct Owner {
        commands: Vec<CommandId>,
        disposed: Cell<u32>,
    }

    impl ShellEnvironment for Owner {
        fn exec(
            &self,
            _: ExecRequest,
            _: CancellationToken,
        ) -> LocalBoxFuture<'_, Result<CommandStatus, ShellError>> {
            unreachable!("the environments never run a command")
        }

        fn status(&self, command: &CommandId, _: Reader) -> Result<CommandStatus, ShellError> {
            Err(ShellError::UnknownCommand(command.clone()))
        }

        fn write<'a>(
            &'a self,
            command: &'a CommandId,
            _: Bytes,
            _: Reader,
        ) -> LocalBoxFuture<'a, Result<CommandStatus, ShellError>> {
            Box::pin(async move { Err(ShellError::UnknownCommand(command.clone())) })
        }

        fn abort<'a>(
            &'a self,
            command: &'a CommandId,
            _: Reader,
        ) -> LocalBoxFuture<'a, Result<CommandStatus, ShellError>> {
            Box::pin(async move { Err(ShellError::UnknownCommand(command.clone())) })
        }

        fn release_command<'a>(&'a self, _: &'a CommandId) -> LocalBoxFuture<'a, bool> {
            Box::pin(async { false })
        }

        fn dispose_shell<'a>(&'a self, _: &'a ShellId) -> LocalBoxFuture<'a, bool> {
            Box::pin(async { false })
        }

        fn dispose_all(&self) -> LocalBoxFuture<'_, ()> {
            self.disposed.set(self.disposed.get() + 1);
            Box::pin(async {})
        }

        fn owns_shell(&self, _: &ShellId) -> bool {
            false
        }

        fn owns_command(&self, command: &CommandId) -> bool {
            self.commands.contains(command)
        }
    }

    fn key(name: &str) -> HostKey {
        HostKey::new(name.to_owned())
    }

    fn owning(commands: &[&str]) -> Rc<Owner> {
        Rc::new(Owner {
            commands: commands
                .iter()
                .map(|id| CommandId::try_from(*id).unwrap())
                .collect(),
            disposed: Cell::new(0),
        })
    }

    #[tokio::test(flavor = "local")]
    async fn concurrent_calls_for_one_host_make_one_environment() {
        let environments = Environments::default();
        let made = Cell::new(0);
        let create = || async {
            made.set(made.get() + 1);
            tokio::task::yield_now().await;
            Ok(owning(&[]) as Rc<dyn ShellEnvironment>)
        };
        let (first, second) = join(
            environments.resolve(key("a"), Handle::None, create()),
            environments.resolve(key("a"), Handle::None, create()),
        )
        .await;
        assert_eq!(made.get(), 1);
        assert!(Rc::ptr_eq(&first.unwrap().0, &second.unwrap().0));
    }

    #[tokio::test(flavor = "local")]
    async fn a_handle_belongs_to_the_environment_of_its_host() {
        let environments = Environments::default();
        let command = CommandId::try_from("cmd-a").unwrap();
        let on_a = owning(&["cmd-a"]);
        environments
            .resolve(key("a"), Handle::None, async {
                Ok(on_a.clone() as Rc<dyn ShellEnvironment>)
            })
            .await
            .unwrap();
        let elsewhere = environments
            .resolve(key("b"), Handle::Command(&command), async {
                Ok(owning(&[]) as Rc<dyn ShellEnvironment>)
            })
            .await;
        assert_eq!(
            elsewhere.err().as_deref(),
            Some("Shell handle \"cmd-a\" belongs to a different Host")
        );
        assert!(
            environments
                .resolve(key("a"), Handle::Command(&command), async {
                    unreachable!("made already")
                })
                .await
                .is_ok()
        );
        // A handle two environments claim is refused.
        let twice = Environments::default();
        for name in ["a", "b"] {
            twice
                .resolve(key(name), Handle::None, async {
                    Ok(owning(&["cmd-a"]) as Rc<dyn ShellEnvironment>)
                })
                .await
                .unwrap();
        }
        let refused = twice
            .resolve(key("a"), Handle::Command(&command), async {
                unreachable!()
            })
            .await;
        assert_eq!(
            refused.err().as_deref(),
            Some("Command id \"cmd-a\" is not unique in this session")
        );
    }

    #[tokio::test(flavor = "local")]
    async fn a_dispose_ends_every_environment_and_one_made_during_it() {
        let environments = Environments::default();
        let first = owning(&[]);
        environments
            .resolve(key("a"), Handle::None, async {
                Ok(first.clone() as Rc<dyn ShellEnvironment>)
            })
            .await
            .unwrap();
        let late = owning(&[]);
        let (gate, open) = tokio::sync::oneshot::channel::<()>();
        let making = environments.resolve(key("b"), Handle::None, async {
            let _ = open.await;
            Ok(late.clone() as Rc<dyn ShellEnvironment>)
        });
        let disposing = async {
            tokio::task::yield_now().await;
            environments.dispose().await;
            let _ = gate.send(());
        };
        let (made, ()) = join(making, disposing).await;
        assert!(made.is_err());
        assert_eq!((first.disposed.get(), late.disposed.get()), (1, 1));
        assert!(
            environments
                .resolve(key("a"), Handle::None, async { unreachable!() })
                .await
                .is_err()
        );
    }

    #[tokio::test(flavor = "local", start_paused = true)]
    async fn the_seventh_identical_exec_in_a_row_is_suppressed_until_the_script_or_the_minute_changes()
     {
        let environments = Environments::default();
        let (slot, _) = environments
            .resolve(key("a"), Handle::None, async {
                Ok(owning(&[]) as Rc<dyn ShellEnvironment>)
            })
            .await
            .unwrap();
        for _ in 0..6 {
            assert!(slot.repeated("make").is_none());
        }
        let suppressed = slot.repeated("make").expect("the seventh is suppressed");
        assert!(suppressed.is_error);
        assert_eq!(
            suppressed.view,
            Some(ToolView::RepeatedShellExec {
                script: "make".into(),
                count: 7
            })
        );
        assert!(matches!(
            slot.repeated("make").and_then(|outcome| outcome.view),
            Some(ToolView::RepeatedShellExec { count: 8, .. })
        ));
        assert!(slot.repeated("make test").is_none());
        for _ in 0..5 {
            assert!(slot.repeated("make").is_none());
        }
        tokio::time::advance(REPEAT_WINDOW + Duration::from_millis(1)).await;
        assert!(
            slot.repeated("make").is_none(),
            "a minute later the count starts again"
        );
    }
}

//! A node's shell environments: one per Host the node used, keyed by the Host's
//! key.
//!
//! Concurrent calls for one Host make one environment, and a handle belongs to
//! the environment that made it, so a `commandId` of another Host's environment
//! is refused.
//!
//! Beside them, this module holds what the node's command reports need to know
//! of its commands' ends: whose end a look showed the node and who stopped
//! one.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
};

use demi_host_interface::{CommandSet, HostError, HostKey, PageState, PageView, Seen, ShellEnvironment};
use demi_shared_types::CommandId;
use futures_util::future::join_all;
use tokio::sync::OnceCell;

/// The environments of one node.
#[derive(Default)]
pub struct Environments {
    slots: RefCell<Vec<Rc<Slot>>>,
    disposed: Cell<bool>,
    /// The node's place in the output of each command it looked at through its
    /// whole output, the commands of other nodes among them.
    ///
    /// It is all of the output for a command whose end a look showed the node,
    /// whose end tells it nothing more
    /// (`crates/agent-server/src/server/shell_output.rs`).
    places: RefCell<HashMap<CommandId, Seen>>,
    /// Who stopped each command that was stopped, which its end's report
    /// names.
    stops: RefCell<HashMap<CommandId, Stopper>>,
}

/// Who stopped a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stopper {
    /// The node that ran it, which saw the stop: its end tells nothing more.
    Itself,
    /// The user, from a page.
    User,
    /// Another agent of the conversation, by its number.
    Agent(u64),
}

/// One Host's environment, once made.
pub(super) struct Slot {
    key: HostKey,
    environment: OnceCell<Rc<dyn ShellEnvironment>>,
}

/// A handle a call carries, which must belong to the call's environment.
#[derive(Clone, Copy)]
pub(super) enum Handle<'a> {
    None,
    Command(&'a CommandId),
}

impl Environments {
    /// The environment for the Host `key`, made with `create` unless it was
    /// made before.
    ///
    /// A concurrent call for the same Host waits for the same creation. The
    /// handle `handle` must be this environment's: one another Host's
    /// environment owns is refused, as is one two environments claim.
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
        if let Handle::Command(command) = handle
            && let Some(owner) = self.owner(command)?
            && owner.key != slot.key
        {
            return Err(format!("Command \"{command}\" belongs to a different Host"));
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
        });
        slots.push(slot.clone());
        slot
    }

    /// The slot whose environment owns `command`, when one does.
    fn owner(&self, command: &CommandId) -> Result<Option<Rc<Slot>>, String> {
        let owners: Vec<Rc<Slot>> = self
            .slots
            .borrow()
            .iter()
            .filter(|slot| {
                slot.environment
                    .get()
                    .is_some_and(|environment| environment.owns_command(command))
            })
            .cloned()
            .collect();
        match owners.as_slice() {
            [] => Ok(None),
            [owner] => Ok(Some(owner.clone())),
            _ => Err(format!(
                "Command id \"{command}\" is not unique in this session"
            )),
        }
    }

    /// Ends each command whose start still waits for its Host, in every
    /// environment made so far, as the user's Stop does
    /// (`crates/backend-host-access/src/access.rs`).
    pub async fn stop_waiting(&self) {
        let made: Vec<Rc<dyn ShellEnvironment>> = self
            .slots
            .borrow()
            .iter()
            .filter_map(|slot| slot.environment.get())
            .cloned()
            .collect();
        join_all(made.iter().map(|environment| environment.stop_waiting())).await;
    }

    /// The pages' view of every command the environments made so far hold.
    pub fn page_views(&self) -> Vec<PageView> {
        self.slots
            .borrow()
            .iter()
            .filter_map(|slot| slot.environment.get())
            .flat_map(|environment| environment.page_views())
            .collect()
    }

    /// Gives every environment made so far the node's commands `commands`
    /// (`ShellEnvironment::replace_commands`); one that refuses them keeps
    /// its own, and the last refusal is the answer.
    pub fn replace_commands(&self, commands: &Rc<CommandSet>) -> Result<(), HostError> {
        let mut refused = Ok(());
        for slot in self.slots.borrow().iter() {
            if let Some(environment) = slot.environment.get()
                && let Err(error) = environment.replace_commands(commands)
            {
                refused = Err(error);
            }
        }
        refused
    }

    /// The environment that owns `command`, among those made.
    pub fn owning(&self, command: &CommandId) -> Option<Rc<dyn ShellEnvironment>> {
        self.slots
            .borrow()
            .iter()
            .filter_map(|slot| slot.environment.get())
            .find(|environment| environment.owns_command(command))
            .cloned()
    }

    /// Stops every command of every environment made so far and forgets them,
    /// so the next call on a Host makes a fresh environment there.
    pub async fn end_all(&self) {
        join_all(
            self.forget_all()
                .iter()
                .map(|environment| environment.dispose_all()),
        )
        .await;
    }

    /// Asks every command of every environment made so far to stop, without
    /// waiting for its end, and hands those environments over.
    ///
    /// The node forgets them, as [`end_all`](Self::end_all) does, so its
    /// disposal lets none of their commands go. Each command reads as stopping
    /// until its runner reports its end, which its environment then records
    /// (`crates/agent-server/src/server/tree/supervisor.rs`).
    pub async fn stop_all(&self) -> Vec<Rc<dyn ShellEnvironment>> {
        let environments = self.forget_all();
        for environment in &environments {
            for view in environment.page_views() {
                if matches!(view.state, PageState::Running | PageState::Waiting { .. }) {
                    // An environment refuses only a command it does not
                    // hold, and it holds every command its views show.
                    let _ = environment.abort(&view.command_id).await;
                }
            }
        }
        environments
    }

    /// Forgets every environment made so far, so the next call on a Host
    /// makes a fresh one there.
    fn forget_all(&self) -> Vec<Rc<dyn ShellEnvironment>> {
        self.slots
            .borrow_mut()
            .drain(..)
            .filter_map(|slot| slot.environment.get().cloned())
            .collect()
    }

    /// A look showed the node `command`'s end, and all of its output.
    pub fn saw_end(&self, command: &CommandId) {
        self.set_place(command, Seen::ALL);
    }

    /// Whether a look showed the node `command`'s end.
    pub fn end_seen(&self, command: &CommandId) -> bool {
        self.place(command) == Seen::ALL
    }

    /// The node's place in `command`'s whole output: none of it before its
    /// first look.
    pub fn place(&self, command: &CommandId) -> Seen {
        self.places.borrow().get(command).copied().unwrap_or_default()
    }

    /// The node's place in `command`'s whole output, when it has one.
    pub fn known_place(&self, command: &CommandId) -> Option<Seen> {
        self.places.borrow().get(command).copied()
    }

    /// Moves the node's place in `command`'s whole output to `seen`.
    pub fn set_place(&self, command: &CommandId, seen: Seen) {
        self.places.borrow_mut().insert(command.clone(), seen);
    }

    /// `stopper` stops `command`, which the node's environments hold.
    pub fn stopped_by(&self, command: &CommandId, stopper: Stopper) {
        self.stops.borrow_mut().insert(command.clone(), stopper);
    }

    /// Who stopped `command`, when it was stopped by a page or an agent.
    pub fn stopper(&self, command: &CommandId) -> Option<Stopper> {
        self.stops.borrow().get(command).copied()
    }

    /// Lets go of every command of every environment, which runs on and
    /// which the node takes up again when it is restored; an environment
    /// made from now on stops its commands.
    pub async fn dispose(&self) {
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
                .map(|environment| environment.detach_all()),
        )
        .await;
    }
}

/// What a call answers once the node is being disposed.
const DISPOSED: &str = "The node's shells are closed";

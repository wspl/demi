//! A viewer's input, delivered to the source it watches in the order it
//! arrives, beside the agent's own input.
//!
//! Keys, buttons and fingers held down are this viewer's own: its end, a change
//! of what it watches, or a release it asks for lets go of only them. The
//! source decides what the input means and what letting go of it takes.

use std::{
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use tokio::sync::mpsc;
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use demi_live_view_protocol::{PointerAction, PointerKind, ViewerInput};

/// Input the viewer sent and the task has not delivered yet. Past this the
/// source is not taking input: what is queued is discarded and held input is
/// released, rather than delivered all at once later, and a source that has
/// stopped taking input cannot grow memory.
const QUEUE: usize = 256;

pub enum Item<M, T> {
    Message(M),
    /// The viewer watches another source, or none.
    Watch(Option<T>),
    /// Release what this viewer holds.
    Release,
}

/// What delivers one viewer's input to the source it watches: the source's
/// side of the input task.
pub trait Deliver: Send + 'static {
    /// What the viewer sends that the source acts on in order with its
    /// input, such as a browser's paste.
    type Message: Send + 'static;
    /// What the viewer watches.
    type Target: Send + Sync + 'static;
    /// What the viewer holds down on its target.
    type Held: Default + Send;
    /// The pointing input the source takes, which its view tells the page
    /// (`LiveInputs`): a mouse's pointer, or touches.
    const POINTER: PointerKind;

    /// `message` and the `next` one as one, when `next` only continues it;
    /// else `next` comes back. A source passes its input to [`merged`].
    fn merged(
        message: &Self::Message,
        next: Self::Message,
    ) -> Result<Self::Message, Self::Message>;

    /// Delivers `message` to `target`, noting what it holds down in `held`.
    fn deliver(
        &self,
        target: &Self::Target,
        held: &mut Self::Held,
        message: Self::Message,
    ) -> impl Future<Output = ()> + Send;

    /// Lets go of what `held` holds on `target`.
    fn release(
        &self,
        target: Option<&Self::Target>,
        held: Self::Held,
    ) -> impl Future<Output = ()> + Send;
}

/// The sending side of a viewer's input task.
pub struct Input<D: Deliver> {
    items: mpsc::Sender<Item<D::Message, D::Target>>,
    overflowed: Arc<AtomicBool>,
}

impl<D: Deliver> Input<D> {
    /// Starts the task on `tasks`; it ends once `ended` is cancelled, as when
    /// what the viewer held went with its source, or once the view drops this
    /// and it released what the viewer holds.
    pub fn start(
        tasks: &TaskTracker,
        ended: CancellationToken,
        deliver: D,
    ) -> (Self, tokio::task::JoinHandle<()>) {
        let (items, receiver) = mpsc::channel(QUEUE);
        let overflowed = Arc::new(AtomicBool::new(false));
        let task = tasks.spawn(run(receiver, overflowed.clone(), deliver, ended));
        (Self { items, overflowed }, task)
    }

    /// Queues input, or discards it when the source is not taking any.
    pub fn send(&self, message: D::Message) {
        if self.items.try_send(Item::Message(message)).is_err() {
            self.overflowed.store(true, Ordering::SeqCst);
        }
    }

    /// Changes what is watched or releases held input; these always arrive.
    pub async fn control(&self, item: Item<D::Message, D::Target>) {
        let _ended = self.items.send(item).await;
    }
}

async fn run<D: Deliver>(
    mut items: mpsc::Receiver<Item<D::Message, D::Target>>,
    overflowed: Arc<AtomicBool>,
    deliver: D,
    ended: CancellationToken,
) {
    let mut target: Option<D::Target> = None;
    let mut held = D::Held::default();
    // The first item a merge took that did not merge, handled next.
    let mut ahead: Option<Item<D::Message, D::Target>> = None;
    loop {
        let item = match ahead.take() {
            Some(item) => item,
            None => tokio::select! {
                biased;
                // What the viewer held went with its source.
                _ = ended.cancelled() => return,
                item = items.recv() => match item {
                    Some(item) => item,
                    None => break,
                },
            },
        };
        if overflowed.swap(false, Ordering::SeqCst) {
            // Whatever was queued no longer reflects what the viewer holds.
            let mut pending = vec![item];
            while let Ok(item) = items.try_recv() {
                pending.push(item);
            }
            deliver
                .release(target.as_ref(), std::mem::take(&mut held))
                .await;
            for item in pending {
                match item {
                    Item::Watch(next) => target = next,
                    Item::Message(_) | Item::Release => {}
                }
            }
            continue;
        }
        match item {
            Item::Watch(next) => {
                deliver
                    .release(target.as_ref(), std::mem::take(&mut held))
                    .await;
                target = next;
            }
            Item::Release => {
                deliver
                    .release(target.as_ref(), std::mem::take(&mut held))
                    .await;
            }
            Item::Message(message) => {
                let Some(target) = &target else { continue };
                let (message, next) = newest::<D>(message, &mut items);
                ahead = next;
                deliver.deliver(target, &mut held, message).await;
            }
        }
    }
    deliver
        .release(target.as_ref(), std::mem::take(&mut held))
        .await;
}

/// `message` with the input queued behind it that only continues it.
/// Also returns the first queued item that does
/// not merge, which is handled next.
fn newest<D: Deliver>(
    mut message: D::Message,
    items: &mut mpsc::Receiver<Item<D::Message, D::Target>>,
) -> (D::Message, Option<Item<D::Message, D::Target>>) {
    while let Ok(item) = items.try_recv() {
        let Item::Message(next) = item else {
            return (message, Some(item));
        };
        match D::merged(&message, next) {
            Ok(merged) => message = merged,
            Err(next) => return (message, Some(Item::Message(next))),
        }
    }
    (message, None)
}

/// `input` and the `next` one as one, when `next` only continues it: a
/// wheel turn with the same modifiers adds its deltas and moves the point,
/// and a pointer move with the same buttons and modifiers replaces a move.
/// Any other `next` comes back.
pub fn merged(input: &ViewerInput, next: ViewerInput) -> Result<ViewerInput, ViewerInput> {
    match (input, next) {
        (
            ViewerInput::Wheel {
                delta_x,
                delta_y,
                modifiers,
                ..
            },
            ViewerInput::Wheel {
                x,
                y,
                delta_x: next_x,
                delta_y: next_y,
                modifiers: next_modifiers,
            },
        ) if *modifiers == next_modifiers => Ok(ViewerInput::Wheel {
            x,
            y,
            delta_x: delta_x + next_x,
            delta_y: delta_y + next_y,
            modifiers: next_modifiers,
        }),
        (
            ViewerInput::Pointer {
                action: PointerAction::Move,
                buttons,
                modifiers,
                ..
            },
            next,
        ) if matches!(
            &next,
            ViewerInput::Pointer {
                action: PointerAction::Move,
                buttons: next_buttons,
                modifiers: next_modifiers,
                ..
            } if next_buttons == buttons && next_modifiers == modifiers
        ) =>
        {
            Ok(next)
        }
        (_, next) => Err(next),
    }
}

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
/// released, rather than delivered all at once later.
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

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use demi_live_view_protocol::PointerButton;

    use super::*;

    /// A source that records what reaches it.
    #[derive(Clone, Default)]
    struct Recorder {
        log: Arc<Mutex<Vec<String>>>,
        /// Holds each delivery until a permit lets it go, as a busy page does.
        busy: Option<Arc<tokio::sync::Semaphore>>,
        /// Told each time a delivery starts.
        entered: Arc<tokio::sync::Notify>,
    }

    impl Deliver for Recorder {
        type Message = ViewerInput;
        type Target = &'static str;
        type Held = Vec<String>;
        const POINTER: PointerKind = PointerKind::Mouse;

        fn merged(message: &ViewerInput, next: ViewerInput) -> Result<ViewerInput, ViewerInput> {
            merged(message, next)
        }

        async fn deliver(&self, target: &&'static str, held: &mut Vec<String>, message: ViewerInput) {
            self.entered.notify_one();
            if let Some(busy) = &self.busy {
                busy.acquire().await.expect("the test keeps the semaphore open").forget();
            }
            if let ViewerInput::Key { code, .. } = &message {
                held.push(code.clone());
            }
            self.log.lock().unwrap().push(format!("{target}: {message:?}"));
        }

        async fn release(&self, target: Option<&&'static str>, held: Vec<String>) {
            self.log
                .lock()
                .unwrap()
                .push(format!("release {target:?} {held:?}"));
        }
    }

    fn wheel(y: f64, delta_y: f64, modifiers: u8) -> ViewerInput {
        ViewerInput::Wheel {
            x: 10.0,
            y,
            delta_x: 0.0,
            delta_y,
            modifiers,
        }
    }

    fn pointer(action: PointerAction, x: f64, buttons: u8) -> ViewerInput {
        ViewerInput::Pointer {
            action,
            x,
            y: 5.0,
            button: PointerButton::None,
            buttons,
            click_count: 0,
            modifiers: 0,
        }
    }

    fn key(code: &str) -> ViewerInput {
        ViewerInput::Key {
            action: demi_live_view_protocol::KeyAction::Down,
            key: code.into(),
            code: code.into(),
            key_code: 0,
            modifiers: 0,
            repeat: false,
            location: 0,
            text: None,
            alt_graph: false,
        }
    }

    #[tokio::test]
    async fn queued_wheel_turns_and_moves_reach_the_source_as_the_newest_input() {
        let busy = Arc::new(tokio::sync::Semaphore::new(0));
        let recorder = Recorder {
            busy: Some(busy.clone()),
            ..Recorder::default()
        };
        let log = recorder.log.clone();
        let entered = recorder.entered.clone();
        let tasks = TaskTracker::new();
        let (input, task) = Input::start(&tasks, CancellationToken::new(), recorder);
        input.control(Item::Watch(Some("t1"))).await;
        // The first turn waits for a busy source; a flick, a turn with Shift
        // held, hover moves, a drag's move and a press queue behind it.
        input.send(wheel(90.0, 10.0, 0));
        entered.notified().await;
        for message in [
            wheel(100.0, 40.0, 0),
            wheel(110.0, 40.0, 0),
            wheel(120.0, 30.0, 0),
            wheel(120.0, 40.0, 8),
            pointer(PointerAction::Move, 1.0, 0),
            pointer(PointerAction::Move, 2.0, 0),
            pointer(PointerAction::Move, 3.0, 1),
            pointer(PointerAction::Down, 3.0, 1),
        ] {
            input.send(message);
        }
        busy.add_permits(100);
        drop(input);
        task.await.unwrap();
        let log = log.lock().unwrap().clone();
        let delivered: Vec<&str> = log
            .iter()
            .map(String::as_str)
            .filter(|line| !line.starts_with("release"))
            .collect();
        assert_eq!(delivered.len(), 6, "{log:#?}");
        assert!(delivered[0].contains("delta_y: 10.0"), "{log:#?}");
        assert!(delivered[1].contains("y: 120.0") && delivered[1].contains("delta_y: 110.0"), "{log:#?}");
        assert!(delivered[2].contains("delta_y: 40.0, modifiers: 8"), "{log:#?}");
        assert!(delivered[3].contains("Move, x: 2.0") && delivered[3].contains("buttons: 0"), "{log:#?}");
        assert!(delivered[4].contains("Move, x: 3.0") && delivered[4].contains("buttons: 1"), "{log:#?}");
        assert!(delivered[5].contains("Down"), "{log:#?}");
    }

    #[tokio::test]
    async fn what_a_viewer_holds_is_let_go_when_it_watches_another_source_and_when_it_leaves() {
        let recorder = Recorder::default();
        let log = recorder.log.clone();
        let tasks = TaskTracker::new();
        let (input, task) = Input::start(&tasks, CancellationToken::new(), recorder);
        input.control(Item::Watch(Some("m1"))).await;
        input.send(key("ShiftLeft"));
        input.control(Item::Watch(Some("m2"))).await;
        input.send(key("KeyA"));
        input.control(Item::Release).await;
        input.send(key("KeyB"));
        drop(input);
        task.await.unwrap();
        let log = log.lock().unwrap().clone();
        let releases: Vec<&String> = log.iter().filter(|line| line.starts_with("release")).collect();
        assert_eq!(
            releases,
            [
                "release None []",
                "release Some(\"m1\") [\"ShiftLeft\"]",
                "release Some(\"m2\") [\"KeyA\"]",
                "release Some(\"m2\") [\"KeyB\"]",
            ],
            "{log:#?}"
        );
    }
}

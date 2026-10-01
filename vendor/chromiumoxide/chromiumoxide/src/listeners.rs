use std::collections::HashMap;
use std::fmt;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::Stream;
use tokio::sync::broadcast;
use tokio_stream::wrappers::{BroadcastStream, errors::BroadcastStreamRecvError};

use chromiumoxide_cdp::cdp::{Event, EventKind, IntoEventKind};
use chromiumoxide_types::MethodId;

pub(crate) const DEFAULT_EVENT_CAPACITY: usize = 16;
const MAX_EVENT_CAPACITY: usize = 256;

/// Bounded event fan-out. Slow listeners never block the CDP handler.
#[derive(Debug, Default)]
pub struct EventListeners {
    listeners: HashMap<MethodId, Vec<EventListener>>,
}

impl EventListeners {
    pub fn add_listener(&mut self, request: EventListenerRequest) {
        self.listeners
            .entry(request.method)
            .or_default()
            .push(EventListener {
                sender: request.sender,
                kind: request.kind,
            });
    }

    pub fn start_send<T: Event>(&mut self, event: T) {
        if let Some(listeners) = self.listeners.get_mut(&T::method_id()) {
            let event: Arc<dyn Event> = Arc::new(event);
            for listener in listeners {
                // A closed receiver has no consumer; poll removes its registration.
                let _ = listener.sender.send(Arc::clone(&event));
            }
        }
    }

    pub fn try_send_custom(
        &mut self,
        method: &str,
        value: serde_json::Value,
    ) -> serde_json::Result<()> {
        if let Some(listeners) = self.listeners.get_mut(method) {
            let converter = listeners.iter().find_map(|listener| match &listener.kind {
                EventKind::Custom(convert) => Some(convert),
                EventKind::BuiltIn => None,
            });
            if let Some(convert) = converter {
                let event = convert(value)?;
                for listener in listeners
                    .iter()
                    .filter(|listener| listener.kind.is_custom())
                {
                    // Sending cannot fail for lag; it fails only after receiver disposal.
                    let _ = listener.sender.send(Arc::clone(&event));
                }
            }
        }
        Ok(())
    }

    pub fn poll(&mut self, _cx: &mut Context<'_>) {
        self.listeners.retain(|_, listeners| {
            listeners.retain(|listener| listener.sender.receiver_count() != 0);
            !listeners.is_empty()
        });
    }
}

pub struct EventListenerRequest {
    sender: broadcast::Sender<Arc<dyn Event>>,
    method: MethodId,
    kind: EventKind,
}

impl fmt::Debug for EventListenerRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EventListenerRequest")
            .field("method", &self.method)
            .field("kind", &self.kind)
            .finish()
    }
}

#[derive(Debug)]
struct EventListener {
    sender: broadcast::Sender<Arc<dyn Event>>,
    kind: EventKind,
}

/// A gap must be handled explicitly; subsequent events are still available.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EventStreamError {
    #[error("CDP event subscription lost {0} events")]
    Lagged(u64),
    #[error("CDP event subscription received an unexpected event type")]
    InvalidType,
}

/// A bounded subscription; dropping it releases its unread events.
pub struct EventStream<T: IntoEventKind> {
    events: BroadcastStream<Arc<dyn Event>>,
    marker: PhantomData<T>,
}

impl<T: IntoEventKind> fmt::Debug for EventStream<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EventStream").finish()
    }
}

impl<T: IntoEventKind> EventStream<T> {
    pub(crate) fn channel(capacity: usize) -> crate::error::Result<(EventListenerRequest, Self)> {
        if !(1..=MAX_EVENT_CAPACITY).contains(&capacity) {
            return Err(crate::error::CdpError::msg(format!(
                "event capacity must be in 1..={MAX_EVENT_CAPACITY}"
            )));
        }
        let (sender, receiver) = broadcast::channel(capacity);
        Ok((
            EventListenerRequest {
                sender,
                method: T::method_id(),
                kind: T::event_kind(),
            },
            Self {
                events: BroadcastStream::new(receiver),
                marker: PhantomData,
            },
        ))
    }
}

impl<T: IntoEventKind + Unpin> Stream for EventStream<T> {
    type Item = Result<Arc<T>, EventStreamError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match Pin::new(&mut self.get_mut().events).poll_next(cx) {
            Poll::Ready(Some(Ok(event))) => Poll::Ready(Some(
                event
                    .into_any_arc()
                    .downcast()
                    .map_err(|_| EventStreamError::InvalidType),
            )),
            Poll::Ready(Some(Err(BroadcastStreamRecvError::Lagged(count)))) => {
                Poll::Ready(Some(Err(EventStreamError::Lagged(count))))
            }
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

#[cfg(test)]
mod tests {
    use chromiumoxide_cdp::cdp::browser_protocol::animation::EventAnimationCanceled;
    use futures::{StreamExt, task::noop_waker_ref};

    use super::*;

    #[tokio::test]
    async fn slow_listener_reports_loss_and_keeps_latest_events() {
        let (request, mut events) = EventStream::<EventAnimationCanceled>::channel(2).unwrap();
        let mut listeners = EventListeners::default();
        listeners.add_listener(request);
        for index in 0..1000 {
            listeners.start_send(EventAnimationCanceled {
                id: index.to_string(),
            });
        }
        assert_eq!(
            events.next().await.unwrap().unwrap_err(),
            EventStreamError::Lagged(998)
        );
        assert_eq!(events.next().await.unwrap().unwrap().id, "998");
        assert_eq!(events.next().await.unwrap().unwrap().id, "999");
        drop(events);
        listeners.poll(&mut Context::from_waker(noop_waker_ref()));
        assert!(listeners.listeners.is_empty());
    }

    #[tokio::test]
    async fn lagging_listener_does_not_block_another_listener() {
        let (slow, mut slow_events) = EventStream::<EventAnimationCanceled>::channel(1).unwrap();
        let (fast, mut fast_events) = EventStream::<EventAnimationCanceled>::channel(1).unwrap();
        let mut listeners = EventListeners::default();
        listeners.add_listener(slow);
        listeners.add_listener(fast);
        for index in 0..1000 {
            listeners.start_send(EventAnimationCanceled {
                id: index.to_string(),
            });
            assert_eq!(
                fast_events.next().await.unwrap().unwrap().id,
                index.to_string()
            );
        }
        assert_eq!(
            slow_events.next().await.unwrap().unwrap_err(),
            EventStreamError::Lagged(999)
        );
        assert_eq!(slow_events.next().await.unwrap().unwrap().id, "999");
    }

    #[test]
    fn capacity_is_bounded() {
        assert!(EventStream::<EventAnimationCanceled>::channel(0).is_err());
        assert!(EventStream::<EventAnimationCanceled>::channel(257).is_err());
    }
}

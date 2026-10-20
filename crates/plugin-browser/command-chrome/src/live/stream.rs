//! A watched tab as a source of pictures: the live view's stream
//! (`demi_live_view::stream`) shares one capture of it among its viewers.
//!
//! The capture follows the tab's viewport. A resize of the tab ends its capture
//! before the page changes, and the next starts once the resize is published
//! (`CaptureGate`). The hub says when its layout has sized the tab, before
//! which nothing is captured.

use std::time::Duration;

use tokio::sync::watch;
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use demi_live_view::stream::{self, Change, StartError};

use crate::driver::capture::{Capture, CaptureChannel, CaptureEvent, Gated};
use crate::driver::operation::BrowserError;
use crate::live::protocol::BrowserViewport;
use crate::tabs::{
    tab::BrowserTab,
    viewport::{Viewports, pixels},
};

pub(crate) type StreamHandle = stream::StreamHandle<BrowserViewport>;
pub(crate) type StreamView = stream::StreamView<BrowserViewport>;
pub(crate) type Event = stream::Event<BrowserViewport>;

/// Starts the stream of `tab`'s pictures over `captures`.
pub(crate) fn start(tab: BrowserTab, captures: CaptureChannel, tasks: &TaskTracker) -> StreamHandle {
    let source = TabSource {
        viewport: tab.state().viewport.subscribe(),
        gate: tab.state().capture.subscribe(),
        halted: captures.stopped(),
        tab,
        captures,
        tasks: tasks.clone(),
    };
    StreamHandle::start(source, tasks)
}

struct TabSource {
    tab: BrowserTab,
    captures: CaptureChannel,
    viewport: watch::Receiver<Viewports>,
    gate: watch::Receiver<Gated>,
    /// Why captures stopped until a viewer asks again, while they do.
    halted: watch::Receiver<Option<String>>,
    tasks: TaskTracker,
}

impl stream::Capture for Capture {
    async fn next(&mut self) -> Option<CaptureEvent> {
        self.events.recv().await
    }

    fn ack(&self, sequence: u32, window: u32) {
        Capture::ack(self, sequence, window);
    }

    fn key_frame(&self) {
        Capture::key_frame(self);
    }

    fn encoding(&self, bitrate: u32, fps: u32) {
        Capture::encoding(self, bitrate, fps);
    }
}

impl stream::Source for TabSource {
    type Capture = Capture;
    type Layout = BrowserViewport;

    fn name(&self) -> String {
        self.tab.id().to_string()
    }

    fn ended(&self) -> CancellationToken {
        self.tab.ended().clone()
    }

    fn layout(&mut self, scale: f64) -> (BrowserViewport, (u32, u32)) {
        let current = self.viewport.borrow_and_update().current;
        (current, pixels(&current, scale))
    }

    fn capturable(&mut self) -> bool {
        // A viewport changes only in a resize, which ends the capture: a
        // capture larger than its page raises the page's own pixel ratio.
        !self.gate.borrow_and_update().resizing()
    }

    async fn changed(&mut self, stopped: bool) -> Change {
        tokio::select! {
            _ = self.viewport.changed() => Change::Layout,
            // The gate lives as long as the tab this source holds.
            _ = self.gate.changed() => Change::Layout,
            // A viewer asked again, or the extension came back by itself.
            resumed = self.halted.changed(), if stopped => match resumed {
                // The channel ends with the environment, as the tab does.
                Err(_) => Change::Gone,
                Ok(()) if self.halted.borrow_and_update().is_none() => Change::Resumed,
                Ok(()) => Change::Layout,
            },
        }
    }

    async fn start(
        &mut self,
        size: (u32, u32),
        fps: u32,
        bitrate: u32,
        stop: &CancellationToken,
    ) -> Result<Option<Capture>, StartError> {
        // A resize that began since `capturable` was read changes the gate,
        // which wakes the stream.
        let Some(hold) = self.tab.state().capture.capture() else {
            return Ok(None);
        };
        match self
            .captures
            .start(self.tab.target_id(), size.0, size.1, fps, bitrate, hold, stop)
            .await
        {
            Ok(capture) => Ok(Some(capture)),
            Err(BrowserError::UnsupportedCapability(reason)) => Err(StartError::Unavailable(reason)),
            Err(BrowserError::CaptureStopped(reason)) => Err(StartError::Stopped(reason)),
            Err(error) => Err(StartError::Failed(error.to_string())),
        }
    }

    /// Paints the tab's page once, beside the stream: a still page sends a
    /// new capture no picture until it paints. The paint ends with the tab,
    /// and waits for a page that does not paint at most five seconds.
    fn redraw(&self) {
        let page = self.tab.page().clone();
        let ended = self.tab.ended().clone();
        self.tasks.spawn(async move {
            tokio::select! {
                _ = ended.cancelled() => {}
                _painted = tokio::time::timeout(
                    Duration::from_secs(5),
                    crate::tabs::viewport::paint(&page),
                ) => {}
            }
        });
    }
}

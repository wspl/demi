//! The backend's end of a runner: pipe records, the connection engine with
//! the test as the runner, and a Host, jobs, callbacks and the media its
//! commands return over a real runner.

mod link;
mod media;
mod pipes;
mod runner;

use demi_backend_remote_host::DeviceSink;
use demi_runner_protocol::wire::{PipeEnd, PipeFrame, PipeFrameDecoder};
use futures_util::{StreamExt as _, stream::BoxStream};

/// A sink's response as its device reads it, frame by frame (`runner.md`
/// § Pipes and output).
pub(crate) struct Framed {
    body: BoxStream<'static, bytes::Bytes>,
    frames: PipeFrameDecoder,
}

impl Framed {
    pub(crate) fn new(sink: DeviceSink) -> Self {
        Self {
            body: sink.into_frames(),
            frames: PipeFrameDecoder::new(),
        }
    }

    /// The next frame; none once the body ended.
    pub(crate) async fn frame(&mut self) -> Option<PipeFrame> {
        loop {
            if let Some(frame) = self.frames.next_frame().unwrap() {
                return Some(frame);
            }
            let chunk = self.body.next().await?;
            self.frames.extend(&chunk);
        }
    }

    /// The data frames' bytes up to the end frame, and the end; the body
    /// ends right after it.
    pub(crate) async fn read(mut self) -> (Vec<u8>, PipeEnd) {
        let mut data = Vec::new();
        loop {
            match self.frame().await.expect("an end frame") {
                PipeFrame::Data(bytes) => data.extend_from_slice(&bytes),
                PipeFrame::Idle => {}
                PipeFrame::End(end) => {
                    assert!(self.body.next().await.is_none(), "the body ends with its end frame");
                    return (data, end);
                }
            }
        }
    }
}

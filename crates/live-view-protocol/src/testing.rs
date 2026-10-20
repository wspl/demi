//! The page's side of the pictures for tests (`testing`): a viewer that
//! joins a frame's chunks, which arrive in order on the relay path and in
//! process, and acknowledges each chunk as the page does.

use std::time::Instant;

use crate::{ACK_FRAME, ChunkAck, DecodeError, VideoHeader};

/// Joins chunks into frames and answers each with its acknowledgement.
pub struct Viewer {
    started: Instant,
    partial: Option<(VideoHeader, Vec<u8>)>,
}

impl Default for Viewer {
    fn default() -> Self {
        Self {
            started: Instant::now(),
            partial: None,
        }
    }
}

impl Viewer {
    /// A video chunk's payload: the acknowledgement frame to send back,
    /// and the frame's header and data once its last chunk came.
    pub fn chunk(&mut self, payload: &[u8]) -> Result<(Vec<u8>, Option<(VideoHeader, Vec<u8>)>), DecodeError> {
        let (header, data) = VideoHeader::split(payload)?;
        let ack = ChunkAck {
            generation: header.generation,
            sequence: header.sequence,
            chunk: header.chunk,
            received: self.started.elapsed().as_secs_f64() * 1000.0,
        };
        let mut frame = (1 + ChunkAck::BYTES as u32).to_be_bytes().to_vec();
        frame.push(ACK_FRAME);
        ack.write(&mut frame);
        if header.chunk == 0 {
            self.partial = Some((header.clone(), Vec::new()));
        }
        let whole = match &mut self.partial {
            Some((first, joined)) if first.generation == header.generation && first.sequence == header.sequence => {
                joined.extend_from_slice(data);
                header.chunk + 1 == header.chunks
            }
            // A frame whose first chunk this viewer did not see.
            _ => false,
        };
        Ok((frame, if whole { self.partial.take() } else { None }))
    }
}

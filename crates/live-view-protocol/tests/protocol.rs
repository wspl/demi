//! The live view's own contract: the chunk header's and the
//! acknowledgements' layout, the frames' splitting, and the input a page may
//! send.

use demi_live_view_protocol::{
    ACK_FRAME, CHUNK_BYTES, CONTROL_FRAME, ChunkAck, FrameSplitter, SourceId, VideoHeader, ViewerInput,
};
use serde_json::json;

#[test]
fn a_video_chunk_s_header_has_its_documented_layout() {
    let header = VideoHeader {
        source: SourceId::try_from("t1").unwrap(),
        generation: 2,
        sequence: 9,
        chunk: 3,
        chunks: 4,
        key: true,
        timestamp: 1.5,
        width: 1280,
        height: 720,
    };
    let mut bytes = Vec::new();
    header.write(&mut bytes);
    assert_eq!(bytes.len(), VideoHeader::BYTES);
    assert_eq!(&bytes[..16], b"t1\0\0\0\0\0\0\0\0\0\0\0\0\0\0");
    assert_eq!(&bytes[16..29], [0, 0, 0, 2, 0, 0, 0, 9, 0, 3, 0, 4, 1]);
    assert_eq!(&bytes[40..], [5, 0, 2, 208]);
    // A whole chunk, its length and kind included, fits the chunk size.
    assert_eq!(5 + VideoHeader::BYTES + VideoHeader::CHUNK_DATA, CHUNK_BYTES);
    bytes.extend_from_slice(b"data");
    assert_eq!(VideoHeader::split(&bytes).unwrap(), (header.clone(), &b"data"[..]));
    assert!(VideoHeader::split(&bytes[..43]).is_err());
    // A chunk outside its frame is refused.
    let mut outside = Vec::new();
    VideoHeader { chunk: 4, ..header }.write(&mut outside);
    assert!(VideoHeader::split(&outside).is_err());
    assert!(SourceId::try_from("t1234567890123456").is_err());
    assert!(SourceId::try_from("").is_err());
}

#[test]
fn acknowledgements_have_their_documented_layout() {
    let acks = [
        ChunkAck { generation: 1, sequence: 7, chunk: 0, received: 12.5 },
        ChunkAck { generation: 1, sequence: 7, chunk: 1, received: 13.0 },
    ];
    let mut bytes = Vec::new();
    for ack in &acks {
        ack.write(&mut bytes);
    }
    assert_eq!(bytes.len(), 2 * ChunkAck::BYTES);
    assert_eq!(&bytes[..12], [0, 0, 0, 1, 0, 0, 0, 7, 0, 0, 0, 0]);
    assert_eq!(&bytes[12..20], 12.5f64.to_be_bytes());
    assert_eq!(ChunkAck::split(&bytes).unwrap(), acks);
    assert!(ChunkAck::split(&bytes[..19]).is_err());
    assert!(ChunkAck::split(&[]).is_err());
}

#[test]
fn frames_split_wherever_their_bytes_break() {
    let mut bytes = Vec::new();
    for (kind, payload) in [(CONTROL_FRAME, &b"{}"[..]), (ACK_FRAME, &[0; 20][..])] {
        bytes.extend_from_slice(&(1 + payload.len() as u32).to_be_bytes());
        bytes.push(kind);
        bytes.extend_from_slice(payload);
    }
    let mut splitter = FrameSplitter::default();
    let mut kinds = Vec::new();
    for byte in &bytes {
        splitter.push(&[*byte]);
        while let Some(frame) = splitter.next().unwrap() {
            kinds.push((frame.kind, frame.payload.len()));
        }
    }
    assert_eq!(kinds, [(CONTROL_FRAME, 2), (ACK_FRAME, 20)]);
    assert!(splitter.is_empty());
    splitter.push(&[0, 0, 0, 0]);
    assert!(splitter.next().is_err());
}

#[test]
fn touches_carry_their_contacts_within_the_picture() {
    let decode = |value: serde_json::Value| {
        demi_shared_types::decode_value::<ViewerInput>(value)
    };
    let fingers = |count: usize| {
        (0..count)
            .map(|id| json!({"id": id, "x": 10, "y": 20}))
            .collect::<Vec<_>>()
    };
    assert!(decode(json!({"type": "touch", "action": "start", "contacts": fingers(2)})).is_ok());
    assert!(decode(json!({"type": "touch", "action": "cancel", "contacts": []})).is_ok());
    assert!(decode(json!({"type": "touch", "action": "move", "contacts": fingers(11)})).is_err());
    assert!(
        decode(json!({"type": "touch", "action": "move",
            "contacts": [{"id": 1, "x": 5000, "y": 0}]}))
        .is_err()
    );
}

//! The live view's own contract: the frame header's layout and the input a
//! page may send.

use demi_live_view_protocol::{SourceId, VideoHeader, ViewerInput};
use serde_json::json;

#[test]
fn the_video_header_has_its_documented_layout() {
    let header = VideoHeader {
        source: SourceId::try_from("t1").unwrap(),
        generation: 2,
        sequence: 9,
        key: true,
        timestamp: 1.5,
        width: 1280,
        height: 720,
    };
    let mut bytes = Vec::new();
    header.write(&mut bytes);
    assert_eq!(bytes.len(), VideoHeader::BYTES);
    assert_eq!(&bytes[..16], b"t1\0\0\0\0\0\0\0\0\0\0\0\0\0\0");
    assert_eq!(&bytes[16..25], [0, 0, 0, 2, 0, 0, 0, 9, 1]);
    assert_eq!(&bytes[36..], [5, 0, 2, 208]);
    bytes.extend_from_slice(b"data");
    assert_eq!(VideoHeader::split(&bytes).unwrap(), (header, &b"data"[..]));
    assert!(VideoHeader::split(&bytes[..39]).is_err());
    assert!(SourceId::try_from("t1234567890123456").is_err());
    assert!(SourceId::try_from("").is_err());
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

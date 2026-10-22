//! The live view's frame splitting.

use demi_live_view_protocol::{ACK_FRAME, CONTROL_FRAME, FrameSplitter};

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

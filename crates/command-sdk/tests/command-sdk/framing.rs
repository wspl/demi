use bytes::Bytes;
use demi_command_protocol::{Completion, Record, RecordDecoder};

#[test]
fn decodes_every_fragmentation_boundary_and_preserves_binary() {
    let expected = vec![
        Record::InputPull,
        Record::Stdout(Bytes::from_static(&[0, 255, 13, 10])),
        Record::Stderr(Bytes::from_static(b"diagnostic")),
        Record::Completion(Completion {
            exit_code: 7,
            error: None,
        }),
    ];
    let wire: Vec<u8> = expected
        .iter()
        .flat_map(|record| record.encode().unwrap())
        .collect();
    for chunk_size in 1..=wire.len() {
        let mut decoder = RecordDecoder::default();
        let mut actual = Vec::new();
        for chunk in wire.chunks(chunk_size) {
            let mut bytes = Bytes::copy_from_slice(chunk);
            while !bytes.is_empty() {
                if let Some(record) = decoder.decode(&mut bytes).unwrap() {
                    actual.push(record);
                }
            }
        }
        decoder.finish().unwrap();
        assert_eq!(actual, expected);
    }
}

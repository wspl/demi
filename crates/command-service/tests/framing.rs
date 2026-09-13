use bytes::Bytes;
use demi_command_service::protocol::{
    Completion, MAX_RECORD_BYTES, ProtocolError, Record, RecordDecoder,
};

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

#[test]
fn rejects_oversized_header_before_payload_arrives() {
    let mut header = vec![1];
    header.extend_from_slice(&((MAX_RECORD_BYTES + 1) as u32).to_be_bytes());
    assert!(matches!(
        RecordDecoder::default().decode(&mut header.into()),
        Err(ProtocolError::TooLarge)
    ));
}

#[test]
fn requires_completion_and_rejects_trailing_records() {
    let mut decoder = RecordDecoder::default();
    decoder
        .decode(
            &mut Record::Stdout(Bytes::from_static(b"partial"))
                .encode()
                .unwrap(),
        )
        .unwrap();
    assert!(matches!(decoder.finish(), Err(ProtocolError::Incomplete)));
    let mut decoder = RecordDecoder::default();
    let completion = Record::Completion(Completion {
        exit_code: 0,
        error: None,
    })
    .encode()
    .unwrap();
    decoder.decode(&mut completion.clone()).unwrap();
    assert!(matches!(
        decoder.decode(&mut completion.clone()),
        Err(ProtocolError::AfterCompletion)
    ));
}

#[test]
fn rejects_invalid_completion_exit_codes_at_the_wire_boundary() {
    for code in [-1, 256] {
        let payload = format!("{{\"exitCode\":{code}}}");
        let mut record = vec![3];
        record.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        record.extend_from_slice(payload.as_bytes());
        assert!(RecordDecoder::default().decode(&mut record.into()).is_err());
    }
}

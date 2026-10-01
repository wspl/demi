//! A command's whole output in the runner wire's kept-output records
//! (`runner.md` § Pipes and output): what `job_read` streams, and what the
//! backend stores as the command's output (`storage.md` § Command outputs).

use bytes::Bytes;
use demi_shared_types::StreamKind;
use demi_runner_protocol::wire::{self, KeptRecord, OutputStream, WireBytes, WireError};
use demi_host_interface::{Missing, OutputRecord, WholeOutput};

/// The output `bytes` hold, records of a kept output, which the `missing`
/// bytes follow.
pub fn decode_output(bytes: &[u8], missing: Option<Missing>) -> Result<WholeOutput, WireError> {
    let records = wire::decode_records(bytes)?
        .into_iter()
        .map(|record| match record {
            KeptRecord::Output(stream, bytes) => {
                OutputRecord::Output(stream_kind(stream), Bytes::from(bytes.0))
            }
            KeptRecord::LeftOut(bytes) => OutputRecord::LeftOut(bytes),
        })
        .collect();
    Ok(WholeOutput::new(records, missing))
}

/// `output`'s records, in the kept output's encoding.
pub fn encode_output(output: &WholeOutput) -> Result<Vec<u8>, WireError> {
    let mut encoded = Vec::new();
    for record in output.records() {
        let record = match record {
            OutputRecord::Output(stream, bytes) => {
                KeptRecord::Output(output_stream(*stream), WireBytes(bytes.to_vec()))
            }
            OutputRecord::LeftOut(bytes) => KeptRecord::LeftOut(*bytes),
        };
        encoded.extend(wire::encode_record(&record)?);
    }
    Ok(encoded)
}

pub(crate) fn stream_kind(stream: OutputStream) -> StreamKind {
    match stream {
        OutputStream::Stdout => StreamKind::Stdout,
        OutputStream::Stderr => StreamKind::Stderr,
    }
}

fn output_stream(stream: StreamKind) -> OutputStream {
    match stream {
        StreamKind::Stdout => OutputStream::Stdout,
        StreamKind::Stderr => OutputStream::Stderr,
    }
}

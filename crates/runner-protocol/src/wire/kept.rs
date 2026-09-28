//! A job's kept output (`runner.md` § Pipes and output): every read of its
//! stdout and stderr, in the order the runner read them, one MessagePack
//! record after another, within [`JOB_KEPT_BYTES`]. The runner writes it
//! while the job runs and `job_read` streams it; the backend decodes it here,
//! where it enters, and stores it in the same records.

use serde::{Deserialize, Serialize};

use super::{OutputStream, WireBytes, WireError};

/// The most bytes of records of reads a kept output holds; below it, it holds
/// every read.
pub const JOB_KEPT_BYTES: usize = 16 * 1024 * 1024;

/// Beyond [`JOB_KEPT_BYTES`], the bytes of records kept at the start, and at
/// most those kept at the end.
pub const JOB_KEPT_PART_BYTES: usize = JOB_KEPT_BYTES / 2;

/// The most bytes a kept output takes: [`JOB_KEPT_BYTES`] of reads, and the
/// record between its parts that counts the bytes left out.
pub const JOB_KEPT_READ_BYTES: usize = JOB_KEPT_BYTES + 32;

/// One record of a kept output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeptRecord {
    /// One read of the job's stdout or stderr.
    Output(OutputStream, WireBytes),
    /// How many bytes of output the kept output leaves out here, between its
    /// first part and its last.
    LeftOut(u64),
}

/// The record's bytes, to append to a kept output.
pub fn encode_record(record: &KeptRecord) -> Result<Vec<u8>, WireError> {
    Ok(rmp_serde::to_vec(record)?)
}

/// The records of a kept output. It is refused when a record does not
/// decode, when its reads take more than [`JOB_KEPT_BYTES`], when a read is
/// empty, or when it leaves bytes out in more than one place.
pub fn decode_records(bytes: &[u8]) -> Result<Vec<KeptRecord>, WireError> {
    let mut decoder = rmp_serde::Deserializer::new(std::io::Cursor::new(bytes));
    let mut records = Vec::new();
    let mut kept = 0u64;
    let mut gaps = 0usize;
    while decoder.position() < bytes.len() as u64 {
        let start = decoder.position();
        let record = KeptRecord::deserialize(&mut decoder)?;
        match &record {
            KeptRecord::Output(_, output) if output.0.is_empty() => {
                return Err(WireError::Invalid("the kept output holds an empty read".into()));
            }
            KeptRecord::Output(..) => kept += decoder.position() - start,
            KeptRecord::LeftOut(_) => gaps += 1,
        }
        if kept > JOB_KEPT_BYTES as u64 {
            return Err(WireError::Invalid(format!(
                "the kept output's reads take more than {JOB_KEPT_BYTES} bytes"
            )));
        }
        if gaps > 1 {
            return Err(WireError::Invalid(
                "the kept output leaves bytes out in more than one place".into(),
            ));
        }
        records.push(record);
    }
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A kept output at its bound, with the largest count of bytes left out,
    /// is within what a `job_read` may carry.
    #[test]
    fn the_record_between_the_parts_fits_the_read_bound() {
        let gap = encode_record(&KeptRecord::LeftOut(u64::MAX)).unwrap();
        assert!(JOB_KEPT_BYTES + gap.len() <= JOB_KEPT_READ_BYTES);
    }

    #[test]
    fn records_round_trip_and_a_second_gap_is_refused() {
        let records = [
            KeptRecord::Output(OutputStream::Stdout, WireBytes(b"a".to_vec())),
            KeptRecord::LeftOut(7),
            KeptRecord::Output(OutputStream::Stderr, WireBytes(b"b".to_vec())),
        ];
        let bytes: Vec<u8> = records
            .iter()
            .flat_map(|record| encode_record(record).unwrap())
            .collect();
        assert_eq!(decode_records(&bytes).unwrap(), records);
        let twice = [bytes.clone(), encode_record(&KeptRecord::LeftOut(1)).unwrap()].concat();
        assert!(decode_records(&twice).is_err());
        assert!(decode_records(&bytes[..bytes.len() - 1]).is_err());
    }
}

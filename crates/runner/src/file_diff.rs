//! Line counting shared by working-tree changes and recorded edits.

const BINARY_PROBE_BYTES: usize = 8_000;

fn is_binary(bytes: &[u8]) -> bool {
    bytes[..bytes.len().min(BINARY_PROBE_BYTES)].contains(&0)
}

fn line_count(bytes: &[u8]) -> u64 {
    if bytes.is_empty() {
        return 0;
    }
    let newlines = bytes.iter().filter(|byte| **byte == b'\n').count() as u64;
    if bytes.ends_with(b"\n") {
        newlines
    } else {
        newlines + 1
    }
}

/// Lines added and removed between two sides; nothing for binary or unread content.
pub(crate) fn line_counts(before: Option<&[u8]>, after: Option<&[u8]>) -> (u64, u64) {
    if before.is_some_and(is_binary) || after.is_some_and(is_binary) {
        return (0, 0);
    }
    match (before, after) {
        (None, Some(after)) => (line_count(after), 0),
        (Some(before), None) => (0, line_count(before)),
        (Some(before), Some(after)) => {
            use gix::diff::blob::{Algorithm, Diff, InternedInput, sources::byte_lines};
            let input = InternedInput::new(byte_lines(before), byte_lines(after));
            let diff = Diff::compute(Algorithm::Histogram, &input);
            (
                u64::from(diff.count_additions()),
                u64::from(diff.count_removals()),
            )
        }
        (None, None) => (0, 0),
    }
}

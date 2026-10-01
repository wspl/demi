//! A command's whole output as its readers see it: the line numbers around
//! a gap, and what counts as binary.

use bytes::Bytes;
use demi_shared_types::StreamKind;
use demi_host_interface::{Missing, OutputRecord, Piece, Seen, Streams, WholeOutput};

fn lines<'a>(pieces: impl Iterator<Item = Piece<'a>>) -> Vec<String> {
    pieces
        .map(|piece| match &piece {
            Piece::Line { number, .. } => format!("{number}:{}", piece.text()),
            Piece::Note(note) => note.clone(),
        })
        .collect()
}

/// A line the kept output left bytes out of shows in two parts around
/// the note, both with its number, whichever way it is read; the numbers
/// are those of the bytes `--raw` prints.
#[test]
fn a_gap_inside_a_line_keeps_the_numbering_of_the_raw_bytes() {
    let whole = WholeOutput::new(
        vec![
            OutputRecord::Output(StreamKind::Stdout, Bytes::from_static(b"one\ntw")),
            OutputRecord::LeftOut(9),
            OutputRecord::Output(StreamKind::Stderr, Bytes::from_static(b"o\nthree\n")),
        ],
        Some(Missing {
            bytes: 4,
            reason: "lost with the Host's connection".into(),
        }),
    );
    let text = whole.text(Streams::Both, None, Seen::default());
    assert_eq!(text.bytes(), b"one\ntwo\nthree\n");
    assert_eq!(text.line_count(), 3);
    let forward = [
        "1:one",
        "2:tw",
        "[... 9 bytes left out ...]",
        "2:o",
        "3:three",
        "[... 4 bytes lost with the Host's connection ...]",
    ];
    assert_eq!(lines(text.forward(1)), forward);
    assert_eq!(lines(text.forward(3))[0], "3:three");
    let mut backward = forward.to_vec();
    backward.reverse();
    assert_eq!(lines(text.backward()), backward);
}

#[test]
fn a_text_stdout_cut_by_the_gap_is_not_binary() {
    // The gap cut a character on each side.
    let cut = WholeOutput::new(
        vec![
            OutputRecord::Output(StreamKind::Stdout, Bytes::from_static(b"ok \xc3")),
            OutputRecord::LeftOut(3),
            OutputRecord::Output(StreamKind::Stdout, Bytes::from_static(b"\xbc!")),
        ],
        None,
    );
    assert_eq!(cut.binary_stdout(9, 100), None);
    let binary = WholeOutput::new(
        vec![OutputRecord::Output(
            StreamKind::Stdout,
            Bytes::from_static(b"\xff\x00"),
        )],
        None,
    );
    let found = binary.binary_stdout(2, 100).expect("binary");
    assert!(!found.info.truncated);
    assert_eq!(found.bytes, Bytes::from_static(b"\xff\x00"));
}

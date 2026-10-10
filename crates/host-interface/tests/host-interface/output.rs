//! A command's whole output as its readers see it: the line numbers around
//! a gap, and what counts as binary.

use bytes::Bytes;
use demi_host_interface::{Missing, OutputRecord, Piece, Seen, Streams, WholeOutput};
use demi_shared_types::StreamKind;

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

/// A stdout is binary when its first 8 KiB hold a NUL byte or it begins as
/// a media type, as git and GNU diff decide (`runtime.md` § What `demi file
/// view` shows). Any other stdout is text, and a byte that is not UTF-8 costs
/// one U+FFFD, where before it hid the whole output.
#[test]
fn a_stdout_is_binary_only_with_an_early_nul_or_as_a_media_type() {
    let stdout = |bytes: &[u8]| {
        WholeOutput::new(
            vec![OutputRecord::Output(StreamKind::Stdout, Bytes::copy_from_slice(bytes))],
            None,
        )
    };
    let stray = stdout(b"ok 1\n\xbb raw\nok 3\n");
    assert_eq!(stray.binary_stdout_length(), None);
    let text = stray.text(Streams::Both, None, Seen::default());
    assert_eq!(lines(text.forward(1)), ["1:ok 1", "2:\u{fffd} raw", "3:ok 3"]);
    assert_eq!(stdout(b"a\0b").binary_stdout_length(), Some(3));
    let png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0";
    assert_eq!(stdout(png).binary_stdout_length(), Some(png.len() as u64));
    // A PDF need hold no NUL to be one.
    assert!(stdout(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n").binary_stdout_length().is_some());
    let mut late = vec![b'a'; 8 * 1024];
    late.extend_from_slice(b"\0\n");
    assert_eq!(stdout(&late).binary_stdout_length(), None);
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

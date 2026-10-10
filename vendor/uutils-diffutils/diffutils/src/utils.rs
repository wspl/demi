// This file is part of the uutils diffutils package.
//
// For the full copyright and license information, please view the LICENSE-*
// files that was distributed with this source code.

use crate::params::{Format, Params};
use regex::Regex;
use {std::ffi::OsString, uucore::context::io::Write};
use unicode_width::UnicodeWidthStr;

/// Replace tabs by spaces in the input line.
/// Correctly handle multi-bytes characters.
/// This assumes that line does not contain any line breaks (if it does, the result is undefined).
#[must_use]
pub fn do_expand_tabs(line: &[u8], tabsize: usize) -> Vec<u8> {
    let tab = b'\t';
    let ntabs = line.iter().filter(|c| **c == tab).count();
    if ntabs == 0 {
        return line.to_vec();
    }
    let mut result = Vec::with_capacity(line.len() + ntabs * (tabsize - 1));
    let mut offset = 0;

    let mut iter = line.split(|c| *c == tab).peekable();
    while let Some(chunk) = iter.next() {
        match String::from_utf8(chunk.to_vec()) {
            Ok(s) => offset += UnicodeWidthStr::width(s.as_str()),
            Err(_) => offset += chunk.len(),
        }
        result.extend_from_slice(chunk);
        if iter.peek().is_some() {
            result.resize(result.len() + tabsize - offset % tabsize, b' ');
            offset = 0;
        }
    }

    result
}

/// One step of a line diff: a line only the first file has, one only the
/// second has, or a line both have, given from each.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edit<'a> {
    Left(&'a [u8]),
    Right(&'a [u8]),
    Both(&'a [u8], &'a [u8]),
}

/// What `-i`, `-b` and `-w` compare of `line`, as GNU diff does in the C
/// locale: `-w` drops every white space byte, `-b` makes each run one space
/// and drops a run at the end, and `-i` lowers ASCII letters.
fn line_key(line: &[u8], params: &Params) -> Vec<u8> {
    let ignores_space = params.ignore_all_space || params.ignore_space_change;
    let mut key = Vec::with_capacity(line.len());
    let mut after_space = false;
    for &byte in line {
        if ignores_space && (byte.is_ascii_whitespace() || byte == b'\x0b') {
            after_space = true;
            continue;
        }
        if after_space && !params.ignore_all_space {
            key.push(b' ');
        }
        after_space = false;
        key.push(if params.ignore_case { byte.to_ascii_lowercase() } else { byte });
    }
    key
}

/// The line diff of `expected` and `actual`, comparing lines as `-i`, `-b`
/// and `-w` say, in GNU diff's way: Myers' algorithm in linear space with
/// GNU's preprocessing and its cost limit for expensive inputs (`-d` lifts
/// it), and each change slid as far down as it goes, or to meet a change in
/// the other file. A change deletes before it inserts.
pub fn diff_lines<'a>(expected: &[&'a [u8]], actual: &[&'a [u8]], params: &Params) -> Vec<Edit<'a>> {
    let ignores = params.ignore_case || params.ignore_space_change || params.ignore_all_space;
    let mut interner = imara_diff::Interner::new(expected.len() + actual.len());
    let mut intern = |lines: &[&'a [u8]]| -> Vec<imara_diff::Token> {
        lines
            .iter()
            .map(|&line| {
                interner.intern(if ignores { std::borrow::Cow::Owned(line_key(line, params)) } else { line.into() })
            })
            .collect()
    };
    // GNU sets aside the files' identical first lines, then their identical
    // last lines, keeping as many of each as the context asks for (its
    // horizon), and diffs only the rest, so no change slides further.
    let horizon = match params.format {
        Format::Unified | Format::Context => params.context_count,
        _ => 0,
    };
    let common = expected.len().min(actual.len());
    let prefix = expected.iter().zip(actual).take_while(|(old, new)| old == new).count();
    let prefix = prefix - prefix.min(horizon);
    let suffix = expected.iter().rev().zip(actual.iter().rev()).take(common - prefix).take_while(|(old, new)| old == new).count();
    let suffix = suffix - suffix.min(horizon);
    let before = intern(&expected[prefix..expected.len() - suffix]);
    let after = intern(&actual[prefix..actual.len() - suffix]);
    let algorithm = if params.minimal { imara_diff::Algorithm::MyersMinimal } else { imara_diff::Algorithm::Myers };
    let mut diff = imara_diff::Diff::default();
    diff.compute_with(algorithm, &before, &after, interner.num_tokens());
    let mut changed = [
        (0..before.len() as u32).map(|line| diff.is_removed(line)).collect::<Vec<_>>(),
        (0..after.len() as u32).map(|line| diff.is_added(line)).collect::<Vec<_>>(),
    ];
    shift_boundaries(&mut changed, [&before, &after]);

    let mut edits = Vec::with_capacity(expected.len().max(actual.len()));
    edits.extend(expected[..prefix].iter().zip(actual).map(|(&left, &right)| Edit::Both(left, right)));
    let (expected_body, actual_body) = (&expected[prefix..], &actual[prefix..]);
    let (mut old, mut new) = (0, 0);
    while old < before.len() || new < after.len() {
        if old < before.len() && changed[0][old] {
            edits.push(Edit::Left(expected_body[old]));
            old += 1;
        } else if new < after.len() && changed[1][new] {
            edits.push(Edit::Right(actual_body[new]));
            new += 1;
        } else {
            edits.push(Edit::Both(expected_body[old], actual_body[new]));
            old += 1;
            new += 1;
        }
    }
    edits.extend(expected_body[old..].iter().zip(&actual_body[new..]).map(|(&left, &right)| Edit::Both(left, right)));
    edits
}

/// GNU diff's `shift_boundaries`, line for line: slides each run of changed
/// lines back to merge with the run before it, then forward as far as it
/// goes, and then back again to where it meets a run of the other file.
/// `changed[f][i]` says whether line `i` of file `f` is changed, and
/// `lines[f]` gives the lines as tokens.
fn shift_boundaries(changed: &mut [Vec<bool>; 2], lines: [&[imara_diff::Token]; 2]) {
    // Both files' flags with an unchanged line before and after, as GNU's
    // arrays have, so the scans below may look one line past either end.
    let mut flags: [Vec<bool>; 2] = [0, 1].map(|f| {
        let mut padded = Vec::with_capacity(changed[f].len() + 2);
        padded.push(false);
        padded.extend_from_slice(&changed[f]);
        padded.push(false);
        padded
    });
    for f in 0..2 {
        let [first, second] = &mut flags;
        let (changed, other) = if f == 0 { (first, &*second) } else { (second, &*first) };
        // GNU's line indexes, each one more here for the padding.
        let at = |line: isize| (line + 1) as usize;
        let equivs = lines[f];
        let same = |a: isize, b: isize| equivs[a as usize] == equivs[b as usize];
        let i_end = equivs.len() as isize;
        let (mut i, mut j): (isize, isize) = (0, 0);
        loop {
            while i < i_end && !changed[at(i)] {
                loop {
                    let was = other[at(j)];
                    j += 1;
                    if !was {
                        break;
                    }
                }
                i += 1;
            }
            if i == i_end {
                break;
            }
            let mut start = i;
            loop {
                i += 1;
                if !changed[at(i)] {
                    break;
                }
            }
            while other[at(j)] {
                j += 1;
            }
            let mut corresponding;
            loop {
                let runlength = i - start;
                while start > 0 && same(start - 1, i - 1) {
                    start -= 1;
                    changed[at(start)] = true;
                    i -= 1;
                    changed[at(i)] = false;
                    while changed[at(start - 1)] {
                        start -= 1;
                    }
                    loop {
                        j -= 1;
                        if !other[at(j)] {
                            break;
                        }
                    }
                }
                corresponding = if other[at(j - 1)] { i } else { i_end };
                while i != i_end && same(start, i) {
                    changed[at(start)] = false;
                    start += 1;
                    changed[at(i)] = true;
                    i += 1;
                    while changed[at(i)] {
                        i += 1;
                    }
                    loop {
                        j += 1;
                        if !other[at(j)] {
                            break;
                        }
                        corresponding = i;
                    }
                }
                if runlength == i - start {
                    break;
                }
            }
            while corresponding < i {
                start -= 1;
                changed[at(start)] = true;
                i -= 1;
                changed[at(i)] = false;
                loop {
                    j -= 1;
                    if !other[at(j)] {
                        break;
                    }
                }
            }
        }
    }
    for f in 0..2 {
        let len = changed[f].len();
        changed[f].copy_from_slice(&flags[f][1..=len]);
    }
}

/// Whether `-B` ignores a change made of these lines: all of them are empty.
pub fn is_blank_change<'a>(mut lines: impl Iterator<Item = &'a [u8]>) -> bool {
    lines.all(<[u8]>::is_empty)
}

/// Write a single line to an output stream, expanding tabs to space if necessary.
/// This assumes that line does not contain any line breaks
/// (if it does and tabs are to be expanded to spaces, the result is undefined).
pub fn do_write_line(
    output: &mut Vec<u8>,
    line: &[u8],
    expand_tabs: bool,
    tabsize: usize,
) -> uucore::context::io::Result<()> {
    if expand_tabs {
        output.write_all(do_expand_tabs(line, tabsize).as_slice())
    } else {
        output.write_all(line)
    }
}

/// Retrieves the modification time of the input file specified by file path.
/// A file that does not exist, which `-N` compares as empty, has the epoch,
/// as GNU's; on another error, such as for standard input, it returns the
/// current system time.
pub fn get_modification_time(file_path: &str) -> String {
    use chrono::{DateTime, Local};
    use uucore::context::fs;
    use uucore::context::io::ErrorKind;
    use std::time::SystemTime;

    let modification_time: SystemTime = match fs::metadata(file_path).and_then(|m| m.modified()) {
        Ok(time) => time,
        Err(error) if error.kind() == ErrorKind::NotFound => SystemTime::UNIX_EPOCH,
        Err(_) => SystemTime::now(),
    };

    let modification_time: DateTime<Local> = modification_time.into();
    let modification_time: String = modification_time
        .format("%Y-%m-%d %H:%M:%S%.9f %z")
        .to_string();

    modification_time
}

pub fn format_file_error(
    executable: &OsString,
    filepath: &OsString,
    error: &uucore::context::io::Error,
) -> String {
    // uucore::context::io::Error's display trait outputs "{detail} (os error {code})"
    // but we want only the {detail} (error string) part
    let error_code_re = Regex::new(r"\ \(os\ error\ \d+\)$").unwrap();
    format!(
        "{}: {}: {}",
        executable.to_string_lossy(),
        filepath.to_string_lossy(),
        error_code_re.replace(error.to_string().as_str(), ""),
    )
}

pub fn report_file_error(
    executable: &OsString,
    filepath: &OsString,
    error: &uucore::context::io::Error,
) {
    uucore::context_eprintln!(
        "{}",
        format_file_error(executable, filepath, error)
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    mod expand_tabs {
        use super::*;
        use pretty_assertions::assert_eq;

        fn assert_tab_expansion(line: &str, tabsize: usize, expected: &str) {
            assert_eq!(
                do_expand_tabs(line.as_bytes(), tabsize),
                expected.as_bytes()
            );
        }

        #[test]
        fn basics() {
            assert_tab_expansion("foo barr   baz", 8, "foo barr   baz");
            assert_tab_expansion("foo\tbarr\tbaz", 8, "foo     barr    baz");
            assert_tab_expansion("foo\tbarr\tbaz", 5, "foo  barr baz");
            assert_tab_expansion("foo\tbarr\tbaz", 2, "foo barr  baz");
        }

        #[test]
        fn multibyte_chars() {
            assert_tab_expansion("foo\tépée\tbaz", 8, "foo     épée    baz");
            assert_tab_expansion("foo\t😉\tbaz", 5, "foo  😉   baz");

            // Note: The Woman Scientist emoji (👩‍🔬) is a ZWJ sequence combining
            // the Woman emoji (👩) and the Microscope emoji (🔬). On supported platforms
            // it is displayed as a single emoji and has a print size of 2 columns.
            // Terminal emulators tend to not support this, and display the two emojis
            // side by side, thus accounting for a print size of 4 columns, but the
            // unicode_width crate reports a correct size of 2.
            assert_tab_expansion("foo\t👩‍🔬\tbaz", 6, "foo   👩‍🔬    baz");
        }

        #[test]
        fn invalid_utf8() {
            // [240, 240, 152, 137] is an invalid UTF-8 sequence, so it is handled as 4 bytes
            assert_eq!(
                do_expand_tabs(&[240, 240, 152, 137, 9, 102, 111, 111], 8),
                &[240, 240, 152, 137, 32, 32, 32, 32, 102, 111, 111]
            );
        }
    }

    mod write_line {
        use super::*;
        use pretty_assertions::assert_eq;

        fn assert_line_written(line: &str, expand_tabs: bool, tabsize: usize, expected: &str) {
            let mut output: Vec<u8> = Vec::new();
            assert!(do_write_line(&mut output, line.as_bytes(), expand_tabs, tabsize).is_ok());
            assert_eq!(output, expected.as_bytes());
        }

        #[test]
        fn basics() {
            assert_line_written("foo bar baz", false, 8, "foo bar baz");
            assert_line_written("foo bar\tbaz", false, 8, "foo bar\tbaz");
            assert_line_written("foo bar\tbaz", true, 8, "foo bar baz");
        }
    }

    mod modification_time {
        use super::*;

        #[test]
        fn set_time() {
            use chrono::{DateTime, Local};
            use std::time::SystemTime;
            use tempfile::NamedTempFile;

            let temp = NamedTempFile::new().unwrap();
            // set file modification time equal to current time
            let current = SystemTime::now();
            let _ = temp.as_file().set_modified(current);

            // format current time
            let current: DateTime<Local> = current.into();
            let current: String = current.format("%Y-%m-%d %H:%M:%S%.9f %z").to_string();

            // verify
            assert_eq!(
                current,
                get_modification_time(&temp.path().to_string_lossy())
            );
        }

        #[test]
        fn absent_file() {
            use chrono::{DateTime, Local};
            use std::time::SystemTime;

            let absent_file = "target/utils/absent-file";

            let epoch: DateTime<Local> = SystemTime::UNIX_EPOCH.into();
            let m_time: DateTime<Local> = get_modification_time(absent_file).parse().unwrap();

            assert_eq!(m_time, epoch);
        }
    }
}

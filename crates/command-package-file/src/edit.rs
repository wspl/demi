//! `file.edit` (`commands.md` § Editing files): SEARCH/REPLACE blocks for
//! one or several files, or `--old` and `--new` for one, matched against the
//! files as they were, written together, and reported as the files now
//! read.

use std::{
    borrow::Cow,
    fs,
    ops::Range,
    path::{Path, PathBuf},
};

use demi_command_package_file_protocol::{Block, BlockLine, Choice, Edit};
use demi_command_sdk::edits::Recording;
use similar::{DiffTag, TextDiff};
use tokio_util::sync::CancellationToken;

use crate::{
    changes::{self, Change},
    files::{FileError, Unchosen, check_cancelled, nearest, resolve},
};

/// How many numbered lines the report shows of all the changes together.
const REPORT_LINES: usize = 60;

/// How many unchanged lines between two pieces of a file's report still
/// make them one.
const JOIN_LINES: usize = 2;

/// One file the edit changes, as the agent named it.
struct Planned {
    name: String,
    path: PathBuf,
    /// The text before, none for a file the edit creates.
    before: Option<String>,
    after: String,
}

/// Makes the edit: plans every file's change against the files as they
/// were, writes them together, and reports what changed.
pub(crate) fn edit(
    cwd: &str,
    edit: &Edit,
    cancellation: &CancellationToken,
    recording: Option<&mut Recording>,
) -> Result<String, FileError> {
    let planned = match edit {
        Edit::Text {
            path: name,
            old,
            new,
            choice,
        } => {
            let path = resolve(cwd, name)?;
            let before = read(&path)?;
            let replacement = text_replacement(&before, name, old, new, *choice)?;
            let after = replace_all(&before, vec![replacement], &path)?;
            vec![Planned {
                name: name.clone(),
                path,
                before: Some(before),
                after,
            }]
        }
        Edit::Blocks(files) => {
            // Each file once, in the order stdin first names it, with every
            // block stdin gives it.
            let mut grouped: Vec<(String, PathBuf, Vec<&Block>)> = Vec::new();
            for file in files {
                let path = resolve(cwd, &file.path)?;
                match grouped.iter_mut().find(|(_, known, _)| *known == path) {
                    Some((_, _, blocks)) => blocks.extend(&file.blocks),
                    None => grouped.push((file.path.clone(), path, file.blocks.iter().collect())),
                }
            }
            grouped
                .into_iter()
                .map(|(name, path, blocks)| {
                    check_cancelled(cancellation)?;
                    plan_blocks(name, path, &blocks, cancellation)
                })
                .collect::<Result<_, _>>()?
        }
    };
    let changes: Vec<Change> = planned
        .iter()
        .filter(|file| file.before.as_deref() != Some(file.after.as_str()))
        .map(|file| {
            Ok(Change {
                path: file.path.clone(),
                before: file.before.clone().map(String::into_bytes),
                after: Some(file.after.clone().into_bytes()),
                permissions: match file.before {
                    Some(_) => Some(fs::metadata(&file.path)?.permissions()),
                    None => None,
                },
            })
        })
        .collect::<Result<_, FileError>>()?;
    check_cancelled(cancellation)?;
    changes::commit(&changes, cancellation, recording)?;
    Ok(report(&planned))
}

/// The change `blocks` make to the file `name`, at `path`: the file a block
/// with an empty SEARCH creates, or the existing file with every block's
/// REPLACE in place of what its SEARCH matched.
fn plan_blocks(
    name: String,
    path: PathBuf,
    blocks: &[&Block],
    cancellation: &CancellationToken,
) -> Result<Planned, FileError> {
    if let Some(index) = blocks.iter().position(|block| block.creates()) {
        let block = index + 1;
        if blocks.len() > 1 {
            return Err(FileError::CreateWithOthers { path, block });
        }
        if fs::symlink_metadata(&path).is_ok() {
            return Err(FileError::FileExists { path, block });
        }
        let after = blocks[index]
            .replace
            .iter()
            .map(|line| match line {
                BlockLine::Text(text) => format!("{text}\n"),
                // The protocol refuses a section in a REPLACE whose SEARCH
                // has none.
                BlockLine::Section => unreachable!("a created file's REPLACE holds no section"),
            })
            .collect();
        return Ok(Planned {
            name,
            path,
            before: None,
            after,
        });
    }
    let before = read(&path)?;
    let file = File::of(&before);
    let replacements = blocks
        .iter()
        .enumerate()
        .map(|(index, block)| {
            check_cancelled(cancellation)?;
            if block.search.contains(&BlockLine::Section) {
                file.lines.replacement(index + 1, block, &path)
            } else {
                file.replacement(index + 1, block, &path)
            }
        })
        .collect::<Result<_, _>>()?;
    let after = replace_all(&before, replacements, &path)?;
    Ok(Planned {
        name,
        path,
        before: Some(before),
        after,
    })
}

/// The text of the file at `path`; a file that cannot be read fails naming
/// it, as `demi file edit: <path>: No such file or directory`.
fn read(path: &Path) -> Result<String, FileError> {
    fs::read_to_string(path).map_err(|error| FileError::Unreadable {
        path: path.to_owned(),
        error,
    })
}

/// One planned replacement: the bytes of the file it replaces, its text,
/// and the block it comes from, for a message about two that overlap.
struct Replacement {
    range: Range<usize>,
    text: String,
    block: usize,
}

/// `--old` replaced by `--new`, at the match `choice` names.
fn text_replacement(
    content: &str,
    name: &str,
    old: &str,
    new: &str,
    choice: Choice,
) -> Result<Replacement, FileError> {
    let matches: Vec<_> = content.match_indices(old).map(|(index, _)| index).collect();
    let index = match choice {
        Choice::Occurrence(occurrence) => *matches
            .get(occurrence - 1)
            .ok_or(FileError::OccurrenceOutOfRange(occurrence))?,
        Choice::Context(context) => {
            match nearest(&matches, |index| line_of(content, index).abs_diff(context)) {
                Ok(index) => index,
                Err(Unchosen::Empty) => return Err(FileError::NoMatchNearContext),
                Err(Unchosen::Tie) => {
                    let candidates = matches
                        .iter()
                        .enumerate()
                        .map(|(occurrence, &index)| {
                            format!(
                                "occurrence {} at line {}",
                                occurrence + 1,
                                line_of(content, index)
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("; ");
                    return Err(FileError::AmbiguousContext {
                        context,
                        candidates,
                    });
                }
            }
        }
        Choice::Only => match matches.as_slice() {
            [index] => *index,
            [] => return Err(FileError::NoMatch(name.to_owned())),
            _ => return Err(FileError::MultipleMatches(name.to_owned())),
        },
    };
    // `--old` is the edit's one replacement, so it overlaps no other.
    Ok(Replacement {
        range: index..index + old.len(),
        text: new.to_owned(),
        block: 1,
    })
}

/// `content`, the file at `path`, with each replacement made; two that
/// overlap fail.
fn replace_all(
    content: &str,
    mut replacements: Vec<Replacement>,
    path: &Path,
) -> Result<String, FileError> {
    replacements.sort_by_key(|replacement| replacement.range.start);
    for pair in replacements.windows(2) {
        if pair[1].range.start < pair[0].range.end {
            let mut blocks = [pair[0].block, pair[1].block];
            blocks.sort_unstable();
            return Err(FileError::BlocksOverlap {
                first: blocks[0],
                second: blocks[1],
                path: path.to_owned(),
            });
        }
    }
    let mut updated = String::with_capacity(content.len());
    let mut end = 0;
    for replacement in &replacements {
        updated.push_str(&content[end..replacement.range.start]);
        updated.push_str(&replacement.text);
        end = replacement.range.end;
    }
    updated.push_str(&content[end..]);
    Ok(updated)
}

/// A file as a SEARCH without sections sees it: its text with each CRLF
/// read as LF, since a block's lines come without their CR, and where each
/// dropped CR was, so a match maps back to the file's bytes.
struct File<'a> {
    lines: Lines<'a>,
    text: Cow<'a, str>,
    /// The index in `text` of each LF whose CR was dropped, ascending.
    dropped: Vec<usize>,
}

impl<'a> File<'a> {
    fn of(content: &'a str) -> Self {
        let dropped: Vec<usize> = content
            .match_indices("\r\n")
            .enumerate()
            .map(|(before, (index, _))| index - before)
            .collect();
        let text = if dropped.is_empty() {
            Cow::Borrowed(content)
        } else {
            Cow::Owned(content.replace("\r\n", "\n"))
        };
        Self {
            lines: Lines::of(content),
            text,
            dropped,
        }
    }

    /// The byte of the file at `index` of the text: a CRLF the text reads
    /// as its LF is taken whole.
    fn byte(&self, index: usize) -> usize {
        index + self.dropped.partition_point(|&at| at < index)
    }

    /// The replacement `block`, the `number`th of the file at `path`, makes:
    /// its REPLACE in place of the one place its SEARCH's text occurs,
    /// written with the file's own line endings. An empty REPLACE of a
    /// SEARCH that ends a line takes the line ending too, so no blank line
    /// is left. Occurrences are counted as `str::match_indices` finds them,
    /// so overlapping ones count as one.
    fn replacement(&self, number: usize, block: &Block, path: &Path) -> Result<Replacement, FileError> {
        let search = text_of(&block.search, "\n");
        // An empty text occurs everywhere and nowhere: a SEARCH of one
        // blank line names no place of the file.
        let found: Vec<usize> = if search.is_empty() {
            Vec::new()
        } else {
            self.text
                .match_indices(search.as_str())
                .map(|(index, _)| index)
                .collect()
        };
        let start = match found.as_slice() {
            [start] => *start,
            [] => {
                return Err(FileError::BlockNoMatch {
                    block: number,
                    path: path.to_owned(),
                    closest: self.lines.closest(block),
                });
            }
            _ => {
                return Err(several(
                    number,
                    path,
                    found.iter().map(|&index| line_of(&self.text, index)),
                ));
            }
        };
        let ending = self.lines.ending_from(line_of(&self.text, start) - 1);
        let text = text_of(&block.replace, ending);
        let mut end = start + search.len();
        if text.is_empty() && !search.ends_with('\n') && self.text[end..].starts_with('\n') {
            end += 1;
        }
        Ok(Replacement {
            range: self.byte(start)..self.byte(end),
            text,
            block: number,
        })
    }
}

/// The text lines of a block's SEARCH or REPLACE without sections, joined
/// by `ending`: the text between its markers without its last line ending.
fn text_of(lines: &[BlockLine], ending: &str) -> String {
    lines
        .iter()
        .map(|line| match line {
            BlockLine::Text(text) => text.as_str(),
            // The caller matches a block with sections as lines.
            BlockLine::Section => unreachable!("a block matched as text holds no section"),
        })
        .collect::<Vec<_>>()
        .join(ending)
}

/// The error of the `number`th block of the file at `path`, whose SEARCH
/// occurs at each of `lines`, 1-based and ascending.
fn several(number: usize, path: &Path, lines: impl Iterator<Item = usize>) -> FileError {
    let mut lines: Vec<usize> = lines.collect();
    let count = lines.len();
    lines.dedup();
    let places = match lines.as_slice() {
        [line] => format!("line {line}"),
        [first @ .., last] => format!(
            "lines {} and {last}",
            first.iter().map(usize::to_string).collect::<Vec<_>>().join(", ")
        ),
        [] => unreachable!("a SEARCH that occurs several times occurs somewhere"),
    };
    FileError::BlockMatchesSeveral {
        block: number,
        path: path.to_owned(),
        count,
        places,
    }
}

/// A file's lines: where each starts, where its text ends, and where its
/// line ending ends.
struct Lines<'a> {
    content: &'a str,
    lines: Vec<Line>,
}

struct Line {
    start: usize,
    text_end: usize,
    end: usize,
}

/// Where a SEARCH matches: its lines, and the lines each of its sections
/// stands for, in order.
struct Match {
    lines: Range<usize>,
    sections: Vec<Range<usize>>,
}

impl<'a> Lines<'a> {
    fn of(content: &'a str) -> Self {
        let mut lines = Vec::new();
        let mut start = 0;
        for line in content.split_inclusive('\n') {
            let end = start + line.len();
            let text = line
                .strip_suffix('\n')
                .map_or(line, |text| text.strip_suffix('\r').unwrap_or(text));
            lines.push(Line {
                start,
                text_end: start + text.len(),
                end,
            });
            start = end;
        }
        Self { content, lines }
    }

    fn text(&self, index: usize) -> &'a str {
        let line = &self.lines[index];
        &self.content[line.start..line.text_end]
    }

    fn ending(&self, index: usize) -> &'a str {
        let line = &self.lines[index];
        &self.content[line.text_end..line.end]
    }

    /// The line ending text written at the line `index` takes: that line's,
    /// or the next line's that has one, or the file's first, or LF.
    fn ending_from(&self, index: usize) -> &'a str {
        (index..self.lines.len())
            .chain(0..self.lines.len())
            .map(|index| self.ending(index))
            .find(|ending| !ending.is_empty())
            .unwrap_or("\n")
    }

    /// The bytes of the lines `range`, endings included.
    fn bytes(&self, range: &Range<usize>) -> Range<usize> {
        let start = self
            .lines
            .get(range.start)
            .map_or(self.content.len(), |line| line.start);
        let end = if range.is_empty() {
            start
        } else {
            self.lines[range.end - 1].end
        };
        start..end
    }

    /// Every place `search` matches whole lines, from the top, each after
    /// the one before it ends, so overlapping matches count as one. A
    /// section stands for the fewest lines that let the rest match, and one
    /// the SEARCH opens with for none, so a match starts at its first line
    /// of text.
    fn matches(&self, search: &[BlockLine]) -> Vec<Match> {
        let leading = search
            .iter()
            .take_while(|line| **line == BlockLine::Section)
            .count();
        let mut matches: Vec<Match> = Vec::new();
        for first in 0..=self.lines.len() {
            if matches.last().is_some_and(|last| first < last.lines.end) {
                continue;
            }
            let mut sections = vec![first..first; leading];
            if let Some(end) = self.match_from(&search[leading..], first, &mut sections) {
                matches.push(Match {
                    lines: first..end,
                    sections,
                });
            }
        }
        matches
    }

    /// Where `pattern` ends when it matches from the line `at`, its
    /// sections as short as the rest allows; they are pushed to `sections`.
    fn match_from(
        &self,
        pattern: &[BlockLine],
        at: usize,
        sections: &mut Vec<Range<usize>>,
    ) -> Option<usize> {
        match pattern.split_first() {
            None => Some(at),
            Some((BlockLine::Text(text), rest)) => {
                if at < self.lines.len() && self.text(at) == text {
                    self.match_from(rest, at + 1, sections)
                } else {
                    None
                }
            }
            Some((BlockLine::Section, rest)) => {
                for end in at..=self.lines.len() {
                    sections.push(at..end);
                    if let Some(found) = self.match_from(rest, end, sections) {
                        return Some(found);
                    }
                    sections.pop();
                }
                None
            }
        }
    }

    /// The replacement `block`, the `number`th of the file at `path`, whose
    /// SEARCH has a section, makes: its REPLACE in place of the whole lines
    /// its SEARCH matches exactly once, each section of the REPLACE as the
    /// file has it and each line written with the file's own line ending.
    fn replacement(&self, number: usize, block: &Block, path: &Path) -> Result<Replacement, FileError> {
        let mut matches = self.matches(&block.search);
        let found = match matches.len() {
            1 => matches.remove(0),
            0 => {
                return Err(FileError::BlockNoMatch {
                    block: number,
                    path: path.to_owned(),
                    closest: self.closest(block),
                });
            }
            _ => {
                return Err(several(
                    number,
                    path,
                    matches.iter().map(|found| found.lines.start + 1),
                ));
            }
        };
        let ending = self.ending_from(found.lines.start);
        let mut text = String::new();
        let mut kept = found.sections.iter();
        for line in &block.replace {
            match line {
                BlockLine::Text(line) => {
                    text.push_str(line);
                    text.push_str(ending);
                }
                BlockLine::Section => {
                    let section = kept.next().expect("the protocol counts the sections");
                    text.push_str(&self.content[self.bytes(section)]);
                }
            }
        }
        // A match that ends the file without a final line ending leaves
        // the replacement without one too.
        if let Some(last) = found.lines.end.checked_sub(1)
            && found.lines.start <= last
            && self.ending(last).is_empty()
            && let Some(trimmed) = text.strip_suffix(ending)
        {
            text.truncate(trimmed.len());
        }
        Ok(Replacement {
            range: self.bytes(&found.lines),
            text,
            block: number,
        })
    }

    /// The lines of the file most like the text lines of `block`'s SEARCH,
    /// numbered: as many lines as it has, from the first that matches its
    /// lines best.
    fn closest(&self, block: &Block) -> String {
        if self.lines.is_empty() {
            return "The file is empty.".to_owned();
        }
        let search: Vec<&str> = block
            .search
            .iter()
            .filter_map(|line| match line {
                BlockLine::Text(text) => Some(text.as_str()),
                BlockLine::Section => None,
            })
            .collect();
        let count = search.len().clamp(1, self.lines.len());
        let score = |first: usize| -> f64 {
            search
                .iter()
                .zip(first..first + count)
                .map(|(line, index)| strsim::sorensen_dice(line.trim(), self.text(index).trim()))
                .sum()
        };
        let mut best = 0;
        let mut best_score = f64::MIN;
        for first in 0..=self.lines.len() - count {
            let score = score(first);
            if score > best_score {
                best = first;
                best_score = score;
            }
        }
        let lines = (best..best + count)
            .map(|index| format!("{}: {}", index + 1, self.text(index)))
            .collect::<Vec<_>>()
            .join("\n");
        let span = match count {
            1 => format!("line is {}", best + 1),
            _ => format!("lines are {}-{}", best + 1, best + count),
        };
        format!("The closest {span}:\n{lines}")
    }
}

/// The 1-based line of the byte at `index`.
fn line_of(content: &str, index: usize) -> usize {
    content[..index]
        .bytes()
        .filter(|&byte| byte == b'\n')
        .count()
        + 1
}

/// What the edit prints: a line for each file, and under an edited one
/// each change as the file now reads, numbered, with a line of context on
/// each side, in pieces set apart by `--`; past [`REPORT_LINES`] numbered
/// lines in all, one line per file saying how to read the rest.
fn report(planned: &[Planned]) -> String {
    let mut out = String::new();
    let mut budget = REPORT_LINES;
    for file in planned {
        let Some(before) = &file.before else {
            let count = file.after.lines().count();
            let unit = if count == 1 { "line" } else { "lines" };
            out.push_str(&format!("Created {} ({count} {unit})\n", file.name));
            continue;
        };
        let diff = TextDiff::from_lines(before.as_str(), file.after.as_str());
        let after: Vec<&str> = file.after.lines().collect();
        let (mut added, mut removed) = (0, 0);
        let mut shown: Vec<Range<usize>> = Vec::new();
        for op in diff.ops() {
            let (tag, old, new) = op.as_tag_tuple();
            if tag == DiffTag::Equal {
                continue;
            }
            removed += old.len();
            added += new.len();
            // The new lines with one of context on each side; a deletion
            // shows the lines on each side of where it was.
            let window = new.start.saturating_sub(1)..(new.end + 1).min(after.len());
            // Pieces that touch or lie within two lines of each other are
            // one, as `diff` joins hunks.
            match shown.last_mut() {
                Some(last) if window.start <= last.end + JOIN_LINES => {
                    last.end = last.end.max(window.end);
                }
                _ => shown.push(window),
            }
        }
        let counts = match (added, removed) {
            (0, 0) => "no change".to_owned(),
            (added, 0) => format!("+{added}"),
            (0, removed) => format!("\u{2212}{removed}"),
            (added, removed) => format!("+{added} \u{2212}{removed}"),
        };
        out.push_str(&format!("Edited {} ({counts})\n", file.name));
        let mut rest: Option<Range<usize>> = None;
        for (piece, window) in shown.iter().enumerate() {
            // Pieces are set apart as `grep -C` sets apart its groups.
            if piece > 0 && budget > 0 {
                out.push_str("--\n");
            }
            for index in window.clone() {
                if budget > 0 {
                    let line = index + 1;
                    match after[index] {
                        "" => out.push_str(&format!("{line:>4}\n")),
                        text => out.push_str(&format!("{line:>4}  {text}\n")),
                    }
                    budget -= 1;
                } else {
                    let rest = rest.get_or_insert(index..index);
                    rest.end = index + 1;
                }
            }
        }
        if let Some(rest) = rest {
            let count: usize = shown
                .iter()
                .map(|window| window.end.min(rest.end).saturating_sub(window.start.max(rest.start)))
                .sum();
            out.push_str(&format!(
                "[\u{2026} {count} more lines changed; read them: sed -n {},{}p {}]\n",
                rest.start + 1,
                rest.end,
                file.name
            ));
        }
    }
    out
}

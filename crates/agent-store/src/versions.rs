//! Which version of its place a row is (`message-editing.md` § Versions):
//! the rows that follow the same row are its siblings; each that replaces
//! none begins a version of its own, in the order the rows were written,
//! and one the session's own rewrite wrote in place of another is of that
//! one's version. Both realizations of the tree store read versions by
//! this rule.

use demi_conversation_socket_protocol::Versions;

/// A row among those that follow the same row: its number and the row it
/// was written in place of, when the session's own rewrite wrote it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sibling {
    pub row: u64,
    pub replaces: Option<u64>,
}

/// The first row of the version `row` is of, among `siblings`.
fn version_start(siblings: &[Sibling], row: u64) -> u64 {
    let mut row = row;
    // Each row replaces an older one, so the walk ends.
    while let Some(replaced) = siblings
        .iter()
        .find(|sibling| sibling.row == row)
        .and_then(|sibling| sibling.replaces)
        .filter(|replaced| siblings.iter().any(|sibling| sibling.row == *replaced))
    {
        row = replaced;
    }
    row
}

/// The first rows of the versions among `siblings`, in the order they were
/// made.
fn starts(siblings: &[Sibling]) -> Vec<u64> {
    let mut starts: Vec<u64> = siblings
        .iter()
        .filter(|sibling| version_start(siblings, sibling.row) == sibling.row)
        .map(|sibling| sibling.row)
        .collect();
    starts.sort_unstable();
    starts
}

/// Where `row` stands among the versions of `siblings`, which hold it;
/// none when they make one version only.
pub fn versions_of(siblings: &[Sibling], row: u64) -> Option<Versions> {
    let starts = starts(siblings);
    if starts.len() < 2 {
        return None;
    }
    let start = version_start(siblings, row);
    let index = starts.iter().position(|first| *first == start)?;
    Some(Versions {
        index: u32::try_from(index + 1).ok()?,
        count: u32::try_from(starts.len()).ok()?,
    })
}

/// The rows of the `version`th version among `siblings`, from 1; none when
/// there is no such version.
pub fn version_rows(siblings: &[Sibling], version: u32) -> Option<Vec<u64>> {
    let starts = starts(siblings);
    let start = *starts.get(usize::try_from(version).ok()?.checked_sub(1)?)?;
    Some(
        siblings
            .iter()
            .filter(|sibling| version_start(siblings, sibling.row) == start)
            .map(|sibling| sibling.row)
            .collect(),
    )
}

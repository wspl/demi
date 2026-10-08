//! The catalog block (`skills.md` § What the model sees): every skill
//! available to a node, sorted by name, in the format other agents and the
//! Agent Skills guide use, in at most 8,000 characters.

use demi_shared_types::xml_escaped;

/// The most characters of a catalog block.
const CATALOG_MAX_CHARS: usize = 8_000;

const HEADER: &str = "The following skills provide specialized instructions for specific tasks.
When a task matches a skill's description, read its SKILL.md at the listed
location before you start, and resolve relative paths in it against the
skill's directory.

<available_skills>
";

const FOOTER: &str = "</available_skills>";

/// The block that tells a model that knew of skills that none is left.
pub(crate) const NONE_AVAILABLE: &str = "No skills are available now.";

/// A skill as the catalog lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Entry {
    pub(crate) name: String,
    pub(crate) description: String,
    /// Where its `SKILL.md` is on the Host.
    pub(crate) location: String,
}

/// The catalog of `entries`, sorted by name. When it is longer than the
/// budget, every description is shortened to the longest common length
/// that fits; when the entries do not fit even without descriptions, the
/// last ones are left out and counted.
pub(crate) fn render(entries: &[Entry]) -> String {
    let mut entries = entries.to_vec();
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    let longest = entries
        .iter()
        .map(|entry| entry.description.chars().count())
        .max()
        .unwrap_or(0);
    let full = block(&entries, Some(longest), 0);
    if chars(&full) <= CATALOG_MAX_CHARS {
        return full;
    }
    // The longest description length that fits, by bisection; one
    // character is the shortest that still says something.
    let fits = |length: usize| chars(&block(&entries, Some(length), 0)) <= CATALOG_MAX_CHARS;
    if fits(1) {
        let (mut low, mut high) = (1, longest);
        while low < high {
            let middle = (low + high).div_ceil(2);
            if fits(middle) {
                low = middle;
            } else {
                high = middle - 1;
            }
        }
        return block(&entries, Some(low), 0);
    }
    let mut left_out = 0;
    loop {
        let shown = &entries[..entries.len() - left_out];
        let text = block(shown, None, left_out);
        if chars(&text) <= CATALOG_MAX_CHARS || shown.is_empty() {
            return text;
        }
        left_out += 1;
    }
}

/// The block of `entries` with each description cut to `length`
/// characters, or none without descriptions, and a line counting the
/// entries `left_out`.
fn block(entries: &[Entry], length: Option<usize>, left_out: usize) -> String {
    let mut text = String::from(HEADER);
    for entry in entries {
        text.push_str("  <skill>\n    <name>");
        text.push_str(&xml_escaped(&entry.name));
        text.push_str("</name>\n");
        if let Some(length) = length {
            text.push_str("    <description>");
            text.push_str(&xml_escaped(&shortened(&entry.description, length)));
            text.push_str("</description>\n");
        }
        text.push_str("    <location>");
        text.push_str(&xml_escaped(&entry.location));
        text.push_str("</location>\n  </skill>\n");
    }
    text.push_str(FOOTER);
    if left_out > 0 {
        text.push_str(&format!("\n{left_out} more skills are not listed."));
    }
    text
}

/// `text` in at most `length` characters: cut at a word and ended with
/// `…` when it is longer.
fn shortened(text: &str, length: usize) -> String {
    if text.chars().count() <= length {
        return text.to_owned();
    }
    let kept: String = text.chars().take(length.saturating_sub(1)).collect();
    let at_word = match kept.rfind(char::is_whitespace) {
        Some(space) if space > 0 => &kept[..space],
        _ => kept.as_str(),
    };
    format!("{}…", at_word.trim_end())
}

fn chars(text: &str) -> usize {
    text.chars().count()
}

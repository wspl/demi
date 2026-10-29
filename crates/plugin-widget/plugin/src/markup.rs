//! Finding a page's tags as a browser would, without parsing it whole: what
//! is not markup is blanked first, so a tag in a comment, a script, a style
//! or a raw-text element is not one, and a tag's end is found past its
//! quoted attributes.

/// The elements whose content is text, not markup.
const RAW_TEXT: [&str; 7] = ["script", "style", "textarea", "title", "template", "noscript", "iframe"];

/// `page` with every comment and the content of every raw-text element
/// blanked to spaces, and lowercased, so a byte offset in it is one in the
/// page.
pub(crate) fn scan(page: &str) -> String {
    let lower = page.to_ascii_lowercase();
    let mut scanned = lower.clone().into_bytes();
    let mut at = 0;
    while at < lower.len() {
        let Some(open) = lower[at..].find('<').map(|found| at + found) else {
            break;
        };
        if lower[open..].starts_with("<!--") {
            let end = lower[open + 4..].find("-->").map_or(lower.len(), |found| open + 4 + found + 3);
            blank(&mut scanned, open, end);
            at = end;
            continue;
        }
        let Some(end) = tag_end_from(&lower, open) else {
            break;
        };
        let name: String = lower[open + 1..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect();
        at = end;
        if RAW_TEXT.contains(&name.as_str()) {
            let content_end = closing_tag(&lower, end, &name).unwrap_or(lower.len());
            blank(&mut scanned, end, content_end);
            at = content_end;
        }
    }
    String::from_utf8(scanned).expect("blanking ASCII keeps the text UTF-8")
}

/// Where the end tag of `name` opens in `lower`, from `from`: `</name`
/// followed by what ends a tag's name, as a browser ends a raw-text element,
/// so `</scriptx` in a script's string does not end the script.
fn closing_tag(lower: &str, from: usize, name: &str) -> Option<usize> {
    let close = format!("</{name}");
    let mut at = from;
    while let Some(found) = lower[at..].find(&close).map(|found| at + found) {
        let after = found + close.len();
        if matches!(lower.as_bytes().get(after), None | Some(b'\t' | b'\n' | b'\x0c' | b'\r' | b' ' | b'/' | b'>')) {
            return Some(found);
        }
        at = after;
    }
    None
}

/// Replaces the bytes of `range` with spaces, keeping multi-byte characters
/// whole: every byte of one is blanked.
fn blank(bytes: &mut [u8], start: usize, end: usize) {
    for byte in &mut bytes[start..end] {
        *byte = b' ';
    }
}

/// The end of the tag that opens at `open` in `scanned`, past its quoted
/// attributes; none for one that never closes.
pub(crate) fn tag_end_from(scanned: &str, open: usize) -> Option<usize> {
    let mut quote = None;
    for (offset, byte) in scanned.as_bytes()[open + 1..].iter().enumerate() {
        match (quote, byte) {
            (Some(q), b) if *b == q => quote = None,
            (Some(_), _) => {}
            (None, b'"' | b'\'') => quote = Some(*byte),
            (None, b'>') => return Some(open + 1 + offset + 1),
            (None, _) => {}
        }
    }
    None
}

/// The first tag named `name` in `scanned`, as its start and end; none when
/// the page has none. The page is read tag by tag, so a tag inside another's
/// quoted attribute is not one.
pub(crate) fn find_tag(scanned: &str, name: &str) -> Option<(usize, usize)> {
    let mut at = 0;
    while let Some(start) = scanned[at..].find('<').map(|found| at + found) {
        let Some(end) = tag_end_from(scanned, start) else {
            return None;
        };
        let found: String = scanned[start + 1..]
            .chars()
            .take_while(|c| !c.is_ascii_whitespace() && *c != '>' && *c != '/')
            .collect();
        if found == name {
            return Some((start, end));
        }
        at = end;
    }
    None
}

/// The value of the attribute `name` of the tag `tag`, from its `<` to its
/// `>`, quoted or not; none when it has no such attribute.
pub(crate) fn attribute(tag: &str, name: &str) -> Option<String> {
    let body = tag.trim_start_matches('<').trim_end_matches('>');
    // Past the tag's own name.
    let mut rest = body.trim_start_matches(|c: char| !c.is_ascii_whitespace() && c != '/');
    loop {
        rest = rest.trim_start_matches(|c: char| c.is_ascii_whitespace() || c == '/');
        if rest.is_empty() {
            return None;
        }
        let key_end = rest
            .find(|c: char| c.is_ascii_whitespace() || c == '=' || c == '/')
            .unwrap_or(rest.len());
        let key = &rest[..key_end];
        rest = rest[key_end..].trim_start();
        let value = if let Some(after) = rest.strip_prefix('=') {
            let after = after.trim_start();
            let (value, tail) = match after.chars().next() {
                Some(quote @ ('"' | '\'')) => {
                    let inner = &after[1..];
                    let close = inner.find(quote).unwrap_or(inner.len());
                    (&inner[..close], inner.get(close + 1..).unwrap_or(""))
                }
                _ => {
                    let end = after.find(|c: char| c.is_ascii_whitespace()).unwrap_or(after.len());
                    (&after[..end], &after[end..])
                }
            };
            rest = tail;
            value
        } else {
            ""
        };
        if key.eq_ignore_ascii_case(name) {
            return Some(value.to_owned());
        }
    }
}

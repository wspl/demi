//! The function bodies an eval's script may stand for (`browser.md`
//! § Evaluation). A script holds statements, as a console takes them, and
//! eval returns the value of the last one. Chrome's read-only evaluation
//! keeps that value only for a script run at the top level, where it cannot
//! see the element a target binds, and runs no `eval` or `Function`, which
//! it counts as side effects; so the script runs as a function body that
//! returns its last statement. Without a JavaScript parser, the last
//! statement is found by trying where it may begin, from the end: Chrome
//! refuses a wrong guess as a syntax error before running anything.

/// The bodies to try in order: the last statement returned after each of
/// the latest places a statement may begin, then the whole script as one
/// expression, then the whole script returning nothing, as a script that
/// ends with a declaration or a loop does. The last body is the script as
/// it is, so its syntax error is the one to report.
pub(crate) fn bodies(script: &str) -> Vec<String> {
    let mut bodies: Vec<String> = starts(script)
        .into_iter()
        .rev()
        .filter_map(|start| {
            let (before, last) = script.split_at(start);
            let last = last.trim().trim_end_matches(';').trim_end();
            (!last.is_empty() && !before.trim().is_empty())
                .then(|| format!("{before}\nreturn (\n{last}\n);"))
        })
        .take(MAX_SPLITS)
        .collect();
    let whole = script.trim().trim_end_matches(';').trim_end();
    if !whole.is_empty() {
        bodies.push(format!("return (\n{whole}\n);"));
    }
    bodies.push(format!("{script}\nreturn undefined;"));
    bodies
}

/// How many places a last statement may begin at are tried.
const MAX_SPLITS: usize = 4;

/// The byte offsets where a top-level statement may begin: after a `;` or
/// a line break outside brackets, strings, template literals, comments and
/// regular expressions.
fn starts(script: &str) -> Vec<usize> {
    let bytes = script.as_bytes();
    let mut starts = Vec::new();
    let mut depth = 0usize;
    // The last byte that is no space or comment, which tells a regular
    // expression's `/` from a division.
    let mut previous: Option<u8> = None;
    let mut word = String::new();
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        match byte {
            b'/' if bytes.get(index + 1) == Some(&b'/') => {
                index = line_end(bytes, index);
                continue;
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index = find(bytes, index + 2, b"*/").map_or(bytes.len(), |end| end + 2);
                continue;
            }
            b'/' if regex_may_start(previous, &word) => {
                index = regex_end(bytes, index);
                previous = Some(b'/');
                word.clear();
                continue;
            }
            b'\'' | b'"' => {
                index = string_end(bytes, index, byte);
                previous = Some(byte);
                word.clear();
                continue;
            }
            b'`' => {
                index = template_end(bytes, index);
                previous = Some(byte);
                word.clear();
                continue;
            }
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth = depth.saturating_sub(1),
            b';' if depth == 0 => starts.push(index + 1),
            b'\n' if depth == 0 => starts.push(index + 1),
            _ => {}
        }
        if byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$' {
            word.push(char::from(byte));
        } else if !byte.is_ascii_whitespace() {
            word.clear();
        }
        if !byte.is_ascii_whitespace() {
            previous = Some(byte);
        }
        index += 1;
    }
    starts
}

/// Whether a `/` after `previous`, or after the keyword `word`, begins a
/// regular expression rather than a division.
fn regex_may_start(previous: Option<u8>, word: &str) -> bool {
    match previous {
        None => true,
        Some(byte) if byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$' => matches!(
            word,
            "return" | "typeof" | "case" | "in" | "of" | "delete" | "void" | "throw" | "new"
        ),
        Some(byte) => !matches!(byte, b')' | b']' | b'}' | b'\'' | b'"' | b'`' | b'/'),
    }
}

fn line_end(bytes: &[u8], from: usize) -> usize {
    find(bytes, from, b"\n").unwrap_or(bytes.len())
}

fn find(bytes: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    bytes
        .get(from..)?
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|at| from + at)
}

/// The offset after the string that the quote at `from` opens.
fn string_end(bytes: &[u8], from: usize, quote: u8) -> usize {
    let mut index = from + 1;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            byte if byte == quote || byte == b'\n' => return index + 1,
            _ => index += 1,
        }
    }
    bytes.len()
}

/// The offset after the regular expression the `/` at `from` opens.
fn regex_end(bytes: &[u8], from: usize) -> usize {
    let mut index = from + 1;
    let mut class = false;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 1,
            b'[' => class = true,
            b']' => class = false,
            b'/' if !class => {
                index += 1;
                while index < bytes.len() && bytes[index].is_ascii_alphabetic() {
                    index += 1;
                }
                return index;
            }
            b'\n' => return index,
            _ => {}
        }
        index += 1;
    }
    bytes.len()
}

/// The offset after the template literal the backtick at `from` opens,
/// with its substitutions.
fn template_end(bytes: &[u8], from: usize) -> usize {
    let mut index = from + 1;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            b'`' => return index + 1,
            b'$' if bytes.get(index + 1) == Some(&b'{') => {
                let mut depth = 0usize;
                index += 1;
                while index < bytes.len() {
                    match bytes[index] {
                        b'{' => depth += 1,
                        b'}' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        b'\'' | b'"' => {
                            index = string_end(bytes, index, bytes[index]) - 1;
                        }
                        b'`' => index = template_end(bytes, index) - 1,
                        _ => {}
                    }
                    index += 1;
                }
                index += 1;
            }
            _ => index += 1,
        }
    }
    bytes.len()
}

#[cfg(test)]
mod tests {
    use super::bodies;

    #[test]
    fn the_last_statement_is_tried_first_then_the_whole_script() {
        assert_eq!(
            bodies("const ps = [...document.querySelectorAll('p')]; ps.map(p => p.textContent)"),
            [
                "const ps = [...document.querySelectorAll('p')];\nreturn (\nps.map(p => p.textContent)\n);",
                "return (\nconst ps = [...document.querySelectorAll('p')]; ps.map(p => p.textContent)\n);",
                "const ps = [...document.querySelectorAll('p')]; ps.map(p => p.textContent)\nreturn undefined;",
            ]
        );
        assert_eq!(
            bodies("document.title"),
            ["return (\ndocument.title\n);", "document.title\nreturn undefined;"]
        );
    }

    #[test]
    fn strings_comments_regular_expressions_templates_and_brackets_hold_no_statement_start() {
        let script = "const a = 'x;y'; // c;d\nconst r = /;\\//g; const t = `${f({a: 1}); }`;\n[1,\n2].map((x) => {\n return x; })";
        let first = &bodies(script)[0];
        assert!(
            first.ends_with("\nreturn (\n[1,\n2].map((x) => {\n return x; })\n);"),
            "{first}"
        );
        let divided = &bodies("const a = 4 / 2; a / 2")[0];
        assert!(divided.ends_with("\nreturn (\na / 2\n);"), "{divided}");
    }
}

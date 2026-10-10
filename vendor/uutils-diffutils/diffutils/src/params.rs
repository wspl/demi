use std::ffi::OsString;
use std::iter::Peekable;

use regex::Regex;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Format {
    #[default]
    Normal,
    Unified,
    Context,
    Ed,
    SideBySide,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Params {
    pub executable: OsString,
    pub from: OsString,
    pub to: OsString,
    pub format: Format,
    pub context_count: usize,
    pub report_identical_files: bool,
    pub brief: bool,
    pub expand_tabs: bool,
    pub tabsize: usize,
    pub width: usize,
    /// `-r`: compares the subdirectories of two directories too.
    pub recursive: bool,
    /// `-N`: a file that exists on one side only is compared with an empty
    /// file, and a directory with an empty directory.
    pub new_file: bool,
    /// `-a`: binary files are compared as text too.
    pub text: bool,
    /// `-i`: upper and lower case letters are equal.
    pub ignore_case: bool,
    /// `-b`: a run of white space equals any other, and white space at the
    /// end of a line is ignored.
    pub ignore_space_change: bool,
    /// `-w`: white space is ignored.
    pub ignore_all_space: bool,
    /// `-B`: a change whose lines are all empty is ignored.
    pub ignore_blank_lines: bool,
    /// `-d`: the smallest set of changes, however long finding it takes.
    pub minimal: bool,
    /// `-x` and the lines of `-X`'s files: the names a directory comparison
    /// leaves out.
    pub excludes: Vec<glob::Pattern>,
    /// The options as given, which the line before each pair of files a
    /// directory comparison shows repeats, as GNU's `diff -r a/x b/x`.
    pub options: Vec<OsString>,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            executable: OsString::default(),
            from: OsString::default(),
            to: OsString::default(),
            format: Format::default(),
            context_count: 3,
            report_identical_files: false,
            brief: false,
            expand_tabs: false,
            tabsize: 8,
            width: 130,
            recursive: false,
            new_file: false,
            text: false,
            ignore_case: false,
            ignore_space_change: false,
            ignore_all_space: false,
            ignore_blank_lines: false,
            minimal: false,
            excludes: Vec::new(),
            options: Vec::new(),
        }
    }
}

pub fn parse_params<I: Iterator<Item = OsString>>(mut opts: Peekable<I>) -> Result<Params, String> {
    // parse CLI

    let Some(executable) = opts.next() else {
        return Err("Usage: <exe> <from> <to>".to_string());
    };
    let (split, options) = split_options(opts, &executable)?;
    let mut opts = split.into_iter().peekable();
    let mut params = Params {
        executable,
        options,
        ..Default::default()
    };
    let mut from = None;
    let mut to = None;
    let mut format = None;
    let mut context = None;
    let tabsize_re = Regex::new(r"^--tabsize=(?<num>\d+)$").unwrap();
    let width_re = Regex::new(r"--width=(?P<long>\d+)$").unwrap();
    while let Some(param) = opts.next() {
        if param == "--" {
            break;
        }
        if param == "-" {
            if from.is_none() {
                from = Some(param);
            } else if to.is_none() {
                to = Some(param);
            } else {
                return Err(format!(
                    "Usage: {} <from> <to>",
                    params.executable.to_string_lossy()
                ));
            }
            continue;
        }
        if param == "-s" || param == "--report-identical-files" {
            params.report_identical_files = true;
            continue;
        }
        if param == "-q" || param == "--brief" {
            params.brief = true;
            continue;
        }
        // `-NUM`, GNU's obsolete spelling of the context's length.
        if let Some(number) = param.to_str().and_then(|text| text.strip_prefix('-')) {
            if !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit()) {
                context = Some(
                    number
                        .parse::<usize>()
                        .map_err(|_| format!("invalid context length '{number}'"))?,
                );
                continue;
            }
        }
        if param == "-r" || param == "--recursive" {
            params.recursive = true;
            continue;
        }
        if param == "-d" || param == "--minimal" {
            params.minimal = true;
            continue;
        }
        if param == "-a" || param == "--text" {
            params.text = true;
            continue;
        }
        if param == "-i" || param == "--ignore-case" {
            params.ignore_case = true;
            continue;
        }
        if param == "-b" || param == "--ignore-space-change" {
            params.ignore_space_change = true;
            continue;
        }
        if param == "-w" || param == "--ignore-all-space" {
            params.ignore_all_space = true;
            continue;
        }
        if param == "-B" || param == "--ignore-blank-lines" {
            params.ignore_blank_lines = true;
            continue;
        }
        if let Some((option, value)) = option_value(&param, &mut opts, &params.executable)? {
            if option == 'x' {
                params.excludes.push(exclude_pattern(&value.to_string_lossy()));
            } else {
                let file = uucore::context::fs::read(&value).map_err(|error| {
                    crate::utils::format_file_error(&params.executable, &value, &error)
                })?;
                for line in file.split(|&byte| byte == b'\n').filter(|line| !line.is_empty()) {
                    params.excludes.push(exclude_pattern(&String::from_utf8_lossy(line)));
                }
            }
            continue;
        }
        if param == "-N" || param == "--new-file" {
            params.new_file = true;
            continue;
        }
        if param == "-t" || param == "--expand-tabs" {
            params.expand_tabs = true;
            continue;
        }
        if param == "--normal" {
            if format.is_some() && format != Some(Format::Normal) {
                return Err("Conflicting output style options".to_string());
            }
            format = Some(Format::Normal);
            continue;
        }
        if param == "-e" || param == "--ed" {
            if format.is_some() && format != Some(Format::Ed) {
                return Err("Conflicting output style options".to_string());
            }
            format = Some(Format::Ed);
            continue;
        }
        if param == "-y" || param == "--side-by-side" {
            if format.is_some() && format != Some(Format::SideBySide) {
                return Err("Conflicting output style option".to_string());
            }
            format = Some(Format::SideBySide);
            continue;
        }
        if width_re.is_match(param.to_string_lossy().as_ref()) {
            let param = param.into_string().unwrap();
            let width_str: &str = width_re
                .captures(param.as_str())
                .unwrap()
                .name("long")
                .unwrap()
                .as_str();

            params.width = match width_str.parse::<usize>() {
                Ok(num) => {
                    if num == 0 {
                        return Err("invalid width «0»".to_string());
                    }

                    num
                }
                Err(_) => return Err(format!("invalid width «{width_str}»")),
            };
            continue;
        }
        if tabsize_re.is_match(param.to_string_lossy().as_ref()) {
            // Because param matches the regular expression,
            // it is safe to assume it is valid UTF-8.
            let param = param.into_string().unwrap();
            let tabsize_str = tabsize_re
                .captures(param.as_str())
                .unwrap()
                .name("num")
                .unwrap()
                .as_str();
            params.tabsize = match tabsize_str.parse::<usize>() {
                Ok(num) => {
                    if num == 0 {
                        return Err("invalid tabsize «0»".to_string());
                    }

                    num
                }
                Err(_) => return Err(format!("invalid tabsize «{tabsize_str}»")),
            };

            continue;
        }
        let next_param = opts.peek();
        match match_context_diff_params(&param, next_param, format) {
            Ok(DiffStyleMatch {
                is_match,
                context_count,
                next_param_consumed,
            }) => {
                if is_match {
                    format = Some(Format::Context);
                    if context_count.is_some() {
                        context = context_count;
                    }
                    if next_param_consumed {
                        opts.next();
                    }
                    continue;
                }
            }
            Err(error) => return Err(error),
        }
        match match_unified_diff_params(&param, next_param, format) {
            Ok(DiffStyleMatch {
                is_match,
                context_count,
                next_param_consumed,
            }) => {
                if is_match {
                    format = Some(Format::Unified);
                    if context_count.is_some() {
                        context = context_count;
                    }
                    if next_param_consumed {
                        opts.next();
                    }
                    continue;
                }
            }
            Err(error) => return Err(error),
        }
        if param.to_string_lossy().starts_with('-') {
            let executable = params.executable.to_string_lossy();
            let text = param.to_string_lossy();
            let complaint = if text.starts_with("--") {
                format!("unrecognized option '{text}'")
            } else {
                format!("invalid option -- '{}'", &text[1..])
            };
            return Err(format!(
                "{executable}: {complaint}\n{executable}: Try '{executable} --help' for more information."
            ));
        }
        if from.is_none() {
            from = Some(param);
        } else if to.is_none() {
            to = Some(param);
        } else {
            return Err(format!(
                "Usage: {} <from> <to>",
                params.executable.to_string_lossy()
            ));
        }
    }
    params.from = if let Some(from) = from {
        from
    } else if let Some(param) = opts.next() {
        param
    } else {
        return Err(format!(
            "Usage: {} <from> <to>",
            params.executable.to_string_lossy()
        ));
    };
    params.to = if let Some(to) = to {
        to
    } else if let Some(param) = opts.next() {
        param
    } else {
        return Err(format!(
            "Usage: {} <from> <to>",
            params.executable.to_string_lossy()
        ));
    };

    params.format = format.unwrap_or(Format::default());
    if let Some(context_count) = context {
        params.context_count = context_count;
    }
    Ok(params)
}

/// The value of `-x PAT`, `--exclude=PAT`, `-X FILE` or `--exclude-from=FILE`,
/// with the option's letter; none for another argument. A value is the rest
/// of the argument after `=`, or else the next argument.
fn option_value(
    param: &OsString,
    opts: &mut impl Iterator<Item = OsString>,
    executable: &OsString,
) -> Result<Option<(char, OsString)>, String> {
    let Some(text) = param.to_str() else {
        return Ok(None);
    };
    let (letter, attached) = match text.split_once('=') {
        Some(("--exclude", value)) => ('x', Some(value)),
        Some(("--exclude-from", value)) => ('X', Some(value)),
        _ => match text {
            "-x" | "--exclude" => ('x', None),
            "-X" | "--exclude-from" => ('X', None),
            _ => return Ok(None),
        },
    };
    let value = match attached {
        Some(value) => OsString::from(value),
        None => opts.next().ok_or_else(|| {
            let executable = executable.to_string_lossy();
            let complaint = match text.strip_prefix("--") {
                Some(_) => format!("option '{text}' requires an argument"),
                None => format!("option requires an argument -- '{letter}'"),
            };
            format!("{executable}: {complaint}\n{executable}: Try '{executable} --help' for more information.")
        })?,
    };
    Ok(Some((letter, value)))
}

/// A name pattern as GNU's fnmatch reads it; one that is not a valid glob
/// matches only itself.
fn exclude_pattern(text: &str) -> glob::Pattern {
    glob::Pattern::new(text).unwrap_or_else(|_| glob::Pattern::new(&glob::Pattern::escape(text)).unwrap())
}

/// Splits each cluster of short options, such as `-rq` or `-Naur`, into
/// its options, as GNU's getopt reads them, and returns the arguments split
/// with the options as given. `-C`, `-U`, `-x` and `-X` take the rest of
/// their cluster as their value, or the next argument when nothing follows
/// them, as `--exclude` and `--exclude-from` take the next argument, and a
/// run of digits is one `-NUM`, so `-5u` and `-u5` are `-5 -u`.
fn split_options(
    mut args: impl Iterator<Item = OsString>,
    executable: &OsString,
) -> Result<(Vec<OsString>, Vec<OsString>), String> {
    let mut split = Vec::new();
    let mut given = Vec::new();
    while let Some(arg) = args.next() {
        if arg == "--" {
            given.push(arg.clone());
            split.push(arg);
            split.extend(args);
            break;
        }
        let Some(text) = arg.to_str().filter(|text| text.starts_with('-') && *text != "-") else {
            split.push(arg);
            continue;
        };
        given.push(arg.clone());
        let mut takes_next = false;
        if text.starts_with("--") {
            let long = complete_long_option(text, executable)?;
            takes_next = long == "--exclude" || long == "--exclude-from";
            split.push(OsString::from(long));
        } else if text.len() == 2 {
            takes_next = ["-C", "-U", "-x", "-X"].contains(&text);
            split.push(arg.clone());
        } else {
            let mut letters = text.char_indices().skip(1);
            while let Some((index, letter)) = letters.next() {
                let rest = &text[index + letter.len_utf8()..];
                if letter.is_ascii_digit() {
                    let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
                    split.push(OsString::from(format!("-{}", &text[index..index + 1 + digits])));
                    for _ in 0..digits {
                        letters.next();
                    }
                    continue;
                }
                if "CUxX".contains(letter) {
                    takes_next = rest.is_empty();
                    split.push(OsString::from(format!("-{letter}")));
                    if !rest.is_empty() {
                        split.push(OsString::from(rest));
                    }
                    break;
                }
                split.push(OsString::from(format!("-{letter}")));
            }
        }
        if takes_next {
            if let Some(next) = args.next() {
                given.push(next.clone());
                split.push(next);
            }
        }
    }
    Ok((split, given))
}

/// GNU diff 3.12's long options, which an abbreviation is completed against.
const LONG_OPTIONS: &[&str] = &[
    "brief", "changed-group-format", "color", "context", "ed", "exclude", "exclude-from",
    "expand-tabs", "from-file", "help", "horizon-lines", "ifdef", "ignore-all-space",
    "ignore-blank-lines", "ignore-case", "ignore-file-name-case", "ignore-matching-lines",
    "ignore-space-change", "ignore-tab-expansion", "ignore-trailing-space", "initial-tab", "label",
    "left-column", "line-format", "minimal", "new-file", "new-group-format", "new-line-format",
    "no-dereference", "no-ignore-file-name-case", "normal", "old-group-format", "old-line-format",
    "paginate", "palette", "rcs", "recursive", "report-identical-files", "show-c-function",
    "show-function-line", "side-by-side", "speed-large-files", "starting-file",
    "strip-trailing-cr", "suppress-blank-empty", "suppress-common-lines", "tabsize", "text",
    "to-file", "unchanged-group-format", "unchanged-line-format", "unidirectional-new-file",
    "unified", "version", "width",
];

/// Completes a long option abbreviated as GNU's getopt_long allows, such as
/// `--recur` for `--recursive`, keeping its `=VALUE`; an unknown one stays
/// as it is, and an ambiguous one is an error naming the possibilities.
fn complete_long_option(text: &str, executable: &OsString) -> Result<String, String> {
    let (name, value) = match text[2..].split_once('=') {
        Some((name, value)) => (name, Some(value)),
        None => (&text[2..], None),
    };
    if name.is_empty() || LONG_OPTIONS.contains(&name) {
        return Ok(text.to_string());
    }
    let candidates: Vec<&str> = LONG_OPTIONS.iter().copied().filter(|option| option.starts_with(name)).collect();
    match candidates.as_slice() {
        [option] => Ok(match value {
            Some(value) => format!("--{option}={value}"),
            None => format!("--{option}"),
        }),
        [] => Ok(text.to_string()),
        _ => {
            let executable = executable.to_string_lossy();
            let possibilities: Vec<String> = candidates.iter().map(|option| format!("'--{option}'")).collect();
            Err(format!(
                "{executable}: option '{text}' is ambiguous; possibilities: {}\n{executable}: Try '{executable} --help' for more information.",
                possibilities.join(" ")
            ))
        }
    }
}

struct DiffStyleMatch {
    is_match: bool,
    context_count: Option<usize>,
    next_param_consumed: bool,
}

fn match_context_diff_params(
    param: &OsString,
    next_param: Option<&OsString>,
    format: Option<Format>,
) -> Result<DiffStyleMatch, String> {
    const CONTEXT_RE: &str = r"^(-c|-C(?<num1>\d*)|--context(=(?<num2>\d*))?)$";
    let regex = Regex::new(CONTEXT_RE).unwrap();
    let is_match = regex.is_match(param.to_string_lossy().as_ref());
    let mut context_count = None;
    let mut next_param_consumed = false;
    if is_match {
        if format.is_some() && format != Some(Format::Context) {
            return Err("Conflicting output style options".to_string());
        }
        let captures = regex.captures(param.to_str().unwrap()).unwrap();
        let num = captures
            .name("num1")
            .or(captures.name("num2"));
        if let Some(numvalue) = num {
            if !numvalue.as_str().is_empty() {
                context_count = Some(numvalue.as_str().parse::<usize>().unwrap());
            }
        }
        if param == "-C" {
            if let Some(p) = next_param {
                let size_str = p.to_string_lossy();
                match size_str.parse::<usize>() {
                    Ok(context_size) => {
                        context_count = Some(context_size);
                        next_param_consumed = true;
                    }
                    Err(_) => return Err(format!("invalid context length '{size_str}'")),
                }
            }
        }
    }
    Ok(DiffStyleMatch {
        is_match,
        context_count,
        next_param_consumed,
    })
}

fn match_unified_diff_params(
    param: &OsString,
    next_param: Option<&OsString>,
    format: Option<Format>,
) -> Result<DiffStyleMatch, String> {
    const UNIFIED_RE: &str = r"^(-u|-U(?<num1>\d*)|--unified(=(?<num2>\d*))?)$";
    let regex = Regex::new(UNIFIED_RE).unwrap();
    let is_match = regex.is_match(param.to_string_lossy().as_ref());
    let mut context_count = None;
    let mut next_param_consumed = false;
    if is_match {
        if format.is_some() && format != Some(Format::Unified) {
            return Err("Conflicting output style options".to_string());
        }
        let captures = regex.captures(param.to_str().unwrap()).unwrap();
        let num = captures
            .name("num1")
            .or(captures.name("num2"));
        if let Some(numvalue) = num {
            if !numvalue.as_str().is_empty() {
                context_count = Some(numvalue.as_str().parse::<usize>().unwrap());
            }
        }
        if param == "-U" {
            if let Some(p) = next_param {
                let size_str = p.to_string_lossy();
                match size_str.parse::<usize>() {
                    Ok(context_size) => {
                        context_count = Some(context_size);
                        next_param_consumed = true;
                    }
                    Err(_) => return Err(format!("invalid context length '{size_str}'")),
                }
            }
        }
    }
    Ok(DiffStyleMatch {
        is_match,
        context_count,
        next_param_consumed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn os(s: &str) -> OsString {
        OsString::from(s)
    }
    #[test]
    fn basics() {
        assert_eq!(
            Ok(Params {
                executable: os("diff"),
                from: os("foo"),
                to: os("bar"),
                ..Default::default()
            }),
            parse_params(
                [os("diff"), os("foo"), os("bar")]
                    .iter()
                    .cloned()
                    .peekable()
            )
        );
        assert_eq!(
            Ok(Params {
                executable: os("diff"),
                from: os("foo"),
                to: os("bar"),
                ..Default::default()
            }),
            parse_params(
                [os("diff"), os("--normal"), os("foo"), os("bar")]
                    .iter()
                    .cloned()
                    .peekable()
            )
        );
    }
    #[test]
    fn basics_ed() {
        for arg in ["-e", "--ed"] {
            assert_eq!(
                Ok(Params {
                    executable: os("diff"),
                    from: os("foo"),
                    to: os("bar"),
                    format: Format::Ed,
                    ..Default::default()
                }),
                parse_params(
                    [os("diff"), os(arg), os("foo"), os("bar")]
                        .iter()
                        .cloned()
                        .peekable()
                )
            );
        }
    }
    #[test]
    fn context_valid() {
        for args in [vec!["-c"], vec!["--context"], vec!["--context="]] {
            let mut params = vec!["diff"];
            params.extend(args);
            params.extend(["foo", "bar"]);
            assert_eq!(
                Ok(Params {
                    executable: os("diff"),
                    from: os("foo"),
                    to: os("bar"),
                    format: Format::Context,
                    ..Default::default()
                }),
                parse_params(params.iter().map(|x| os(x)).peekable())
            );
        }
        for args in [
            vec!["-c42"],
            vec!["-C42"],
            vec!["-C", "42"],
            vec!["--context=42"],
            vec!["-42c"],
        ] {
            let mut params = vec!["diff"];
            params.extend(args);
            params.extend(["foo", "bar"]);
            assert_eq!(
                Ok(Params {
                    executable: os("diff"),
                    from: os("foo"),
                    to: os("bar"),
                    format: Format::Context,
                    context_count: 42,
                    ..Default::default()
                }),
                parse_params(params.iter().map(|x| os(x)).peekable())
            );
        }
    }
    #[test]
    fn context_invalid() {
        for args in [
            vec!["-c", "42"],
            vec!["-c=42"],
            vec!["-c="],
            vec!["-C"],
            vec!["-C=42"],
            vec!["-C="],
            vec!["--context42"],
            vec!["--context", "42"],
            vec!["-42C"],
        ] {
            let mut params = vec!["diff"];
            params.extend(args);
            params.extend(["foo", "bar"]);
            assert!(parse_params(params.iter().map(|x| os(x)).peekable()).is_err());
        }
    }
    #[test]
    fn unified_valid() {
        for args in [vec!["-u"], vec!["--unified"], vec!["--unified="]] {
            let mut params = vec!["diff"];
            params.extend(args);
            params.extend(["foo", "bar"]);
            assert_eq!(
                Ok(Params {
                    executable: os("diff"),
                    from: os("foo"),
                    to: os("bar"),
                    format: Format::Unified,
                    ..Default::default()
                }),
                parse_params(params.iter().map(|x| os(x)).peekable())
            );
        }
        for args in [
            vec!["-u42"],
            vec!["-U42"],
            vec!["-U", "42"],
            vec!["--unified=42"],
            vec!["-42u"],
        ] {
            let mut params = vec!["diff"];
            params.extend(args);
            params.extend(["foo", "bar"]);
            assert_eq!(
                Ok(Params {
                    executable: os("diff"),
                    from: os("foo"),
                    to: os("bar"),
                    format: Format::Unified,
                    context_count: 42,
                    ..Default::default()
                }),
                parse_params(params.iter().map(|x| os(x)).peekable())
            );
        }
    }
    #[test]
    fn unified_invalid() {
        for args in [
            vec!["-u", "42"],
            vec!["-u=42"],
            vec!["-u="],
            vec!["-U"],
            vec!["-U=42"],
            vec!["-U="],
            vec!["--unified42"],
            vec!["--unified", "42"],
            vec!["-42U"],
        ] {
            let mut params = vec!["diff"];
            params.extend(args);
            params.extend(["foo", "bar"]);
            assert!(parse_params(params.iter().map(|x| os(x)).peekable()).is_err());
        }
    }
    #[test]
    fn context_count() {
        assert_eq!(
            Ok(Params {
                executable: os("diff"),
                from: os("foo"),
                to: os("bar"),
                format: Format::Unified,
                context_count: 54,
                ..Default::default()
            }),
            parse_params(
                [os("diff"), os("-u54"), os("foo"), os("bar")]
                    .iter()
                    .cloned()
                    .peekable()
            )
        );
        assert_eq!(
            Ok(Params {
                executable: os("diff"),
                from: os("foo"),
                to: os("bar"),
                format: Format::Unified,
                context_count: 54,
                ..Default::default()
            }),
            parse_params(
                [os("diff"), os("-U54"), os("foo"), os("bar")]
                    .iter()
                    .cloned()
                    .peekable()
            )
        );
        assert_eq!(
            Ok(Params {
                executable: os("diff"),
                from: os("foo"),
                to: os("bar"),
                format: Format::Unified,
                context_count: 54,
                ..Default::default()
            }),
            parse_params(
                [os("diff"), os("-U"), os("54"), os("foo"), os("bar")]
                    .iter()
                    .cloned()
                    .peekable()
            )
        );
        assert_eq!(
            Ok(Params {
                executable: os("diff"),
                from: os("foo"),
                to: os("bar"),
                format: Format::Context,
                context_count: 54,
                ..Default::default()
            }),
            parse_params(
                [os("diff"), os("-c54"), os("foo"), os("bar")]
                    .iter()
                    .cloned()
                    .peekable()
            )
        );
    }
    #[test]
    fn report_identical_files() {
        assert_eq!(
            Ok(Params {
                executable: os("diff"),
                from: os("foo"),
                to: os("bar"),
                ..Default::default()
            }),
            parse_params(
                [os("diff"), os("foo"), os("bar")]
                    .iter()
                    .cloned()
                    .peekable()
            )
        );
        assert_eq!(
            Ok(Params {
                executable: os("diff"),
                from: os("foo"),
                to: os("bar"),
                report_identical_files: true,
                ..Default::default()
            }),
            parse_params(
                [os("diff"), os("-s"), os("foo"), os("bar")]
                    .iter()
                    .cloned()
                    .peekable()
            )
        );
        assert_eq!(
            Ok(Params {
                executable: os("diff"),
                from: os("foo"),
                to: os("bar"),
                report_identical_files: true,
                ..Default::default()
            }),
            parse_params(
                [
                    os("diff"),
                    os("--report-identical-files"),
                    os("foo"),
                    os("bar"),
                ]
                .iter()
                .cloned()
                .peekable()
            )
        );
    }
    #[test]
    fn brief() {
        assert_eq!(
            Ok(Params {
                executable: os("diff"),
                from: os("foo"),
                to: os("bar"),
                ..Default::default()
            }),
            parse_params(
                [os("diff"), os("foo"), os("bar")]
                    .iter()
                    .cloned()
                    .peekable()
            )
        );
        assert_eq!(
            Ok(Params {
                executable: os("diff"),
                from: os("foo"),
                to: os("bar"),
                brief: true,
                ..Default::default()
            }),
            parse_params(
                [os("diff"), os("-q"), os("foo"), os("bar")]
                    .iter()
                    .cloned()
                    .peekable()
            )
        );
        assert_eq!(
            Ok(Params {
                executable: os("diff"),
                from: os("foo"),
                to: os("bar"),
                brief: true,
                ..Default::default()
            }),
            parse_params(
                [os("diff"), os("--brief"), os("foo"), os("bar"),]
                    .iter()
                    .cloned()
                    .peekable()
            )
        );
    }
    #[test]
    fn expand_tabs() {
        assert_eq!(
            Ok(Params {
                executable: os("diff"),
                from: os("foo"),
                to: os("bar"),
                ..Default::default()
            }),
            parse_params(
                [os("diff"), os("foo"), os("bar")]
                    .iter()
                    .cloned()
                    .peekable()
            )
        );
        for option in ["-t", "--expand-tabs"] {
            assert_eq!(
                Ok(Params {
                    executable: os("diff"),
                    from: os("foo"),
                    to: os("bar"),
                    expand_tabs: true,
                    ..Default::default()
                }),
                parse_params(
                    [os("diff"), os(option), os("foo"), os("bar")]
                        .iter()
                        .cloned()
                        .peekable()
                )
            );
        }
    }
    #[test]
    fn tabsize() {
        assert_eq!(
            Ok(Params {
                executable: os("diff"),
                from: os("foo"),
                to: os("bar"),
                ..Default::default()
            }),
            parse_params(
                [os("diff"), os("foo"), os("bar")]
                    .iter()
                    .cloned()
                    .peekable()
            )
        );
        assert_eq!(
            Ok(Params {
                executable: os("diff"),
                from: os("foo"),
                to: os("bar"),
                tabsize: 1,
                ..Default::default()
            }),
            parse_params(
                [os("diff"), os("--tabsize=1"), os("foo"), os("bar")]
                    .iter()
                    .cloned()
                    .peekable()
            )
        );
        assert_eq!(
            Ok(Params {
                executable: os("diff"),
                from: os("foo"),
                to: os("bar"),
                tabsize: 42,
                ..Default::default()
            }),
            parse_params(
                [os("diff"), os("--tabsize=42"), os("foo"), os("bar")]
                    .iter()
                    .cloned()
                    .peekable()
            )
        );
        assert!(parse_params(
            [os("diff"), os("--tabsize"), os("foo"), os("bar")]
                .iter()
                .cloned()
                .peekable()
        )
        .is_err());
        assert!(parse_params(
            [os("diff"), os("--tabsize="), os("foo"), os("bar")]
                .iter()
                .cloned()
                .peekable()
        )
        .is_err());
        assert!(parse_params(
            [os("diff"), os("--tabsize=r2"), os("foo"), os("bar")]
                .iter()
                .cloned()
                .peekable()
        )
        .is_err());
        assert!(parse_params(
            [os("diff"), os("--tabsize=-1"), os("foo"), os("bar")]
                .iter()
                .cloned()
                .peekable()
        )
        .is_err());
        assert!(parse_params(
            [os("diff"), os("--tabsize=r2"), os("foo"), os("bar")]
                .iter()
                .cloned()
                .peekable()
        )
        .is_err());
        assert!(parse_params(
            [
                os("diff"),
                os("--tabsize=92233720368547758088"),
                os("foo"),
                os("bar")
            ]
            .iter()
            .cloned()
            .peekable()
        )
        .is_err());
    }
    #[test]
    fn double_dash() {
        assert_eq!(
            Ok(Params {
                executable: os("diff"),
                from: os("-g"),
                to: os("-h"),
                ..Default::default()
            }),
            parse_params(
                [os("diff"), os("--"), os("-g"), os("-h")]
                    .iter()
                    .cloned()
                    .peekable()
            )
        );
    }
    #[test]
    fn default_to_stdin() {
        assert_eq!(
            Ok(Params {
                executable: os("diff"),
                from: os("foo"),
                to: os("-"),
                ..Default::default()
            }),
            parse_params([os("diff"), os("foo"), os("-")].iter().cloned().peekable())
        );
        assert_eq!(
            Ok(Params {
                executable: os("diff"),
                from: os("-"),
                to: os("bar"),
                ..Default::default()
            }),
            parse_params([os("diff"), os("-"), os("bar")].iter().cloned().peekable())
        );
        assert_eq!(
            Ok(Params {
                executable: os("diff"),
                from: os("-"),
                to: os("-"),
                ..Default::default()
            }),
            parse_params([os("diff"), os("-"), os("-")].iter().cloned().peekable())
        );
        assert!(parse_params(
            [os("diff"), os("foo"), os("bar"), os("-")]
                .iter()
                .cloned()
                .peekable()
        )
        .is_err());
        assert!(parse_params(
            [os("diff"), os("-"), os("-"), os("-")]
                .iter()
                .cloned()
                .peekable()
        )
        .is_err());
    }
    #[test]
    fn missing_arguments() {
        assert!(parse_params([os("diff")].iter().cloned().peekable()).is_err());
        assert!(parse_params([os("diff"), os("foo")].iter().cloned().peekable()).is_err());
    }
    #[test]
    fn unknown_argument() {
        assert!(parse_params(
            [os("diff"), os("-g"), os("foo"), os("bar")]
                .iter()
                .cloned()
                .peekable()
        )
        .is_err());
        assert!(
            parse_params([os("diff"), os("-g"), os("bar")].iter().cloned().peekable()).is_err()
        );
        assert!(parse_params([os("diff"), os("-g")].iter().cloned().peekable()).is_err());
    }
    #[test]
    fn empty() {
        assert!(parse_params([].iter().cloned().peekable()).is_err());
    }
    #[test]
    fn conflicting_output_styles() {
        for (arg1, arg2) in [
            ("-u", "-c"),
            ("-u", "-e"),
            ("-c", "-u"),
            ("-c", "-U42"),
            ("-u", "--normal"),
            ("--normal", "-e"),
            ("--context", "--normal"),
        ] {
            assert!(parse_params(
                [os("diff"), os(arg1), os(arg2), os("foo"), os("bar")]
                    .iter()
                    .cloned()
                    .peekable()
            )
            .is_err());
        }
    }
}

//! How a script runs where the system cannot execute one itself.
//!
//! Windows starts only programs. A script's first line, `#!` and an
//! interpreter, names what runs it, as Git Bash reads it: the interpreter
//! starts with the line's one optional argument, the script's path and the
//! script's own arguments. The extension never chooses the interpreter, so a
//! `.ps1` file without such a line stays one Windows cannot start, and the
//! command says `powershell -File x.ps1`.
//!
//! A script that names `sh` or `bash`, directly or through `env`, is run by
//! the job's own shell, as a copy of it: Windows has no `/bin/sh`, a `bash` on
//! its `PATH` may be another system's, such as the WSL launcher, and the
//! embedded shell is the one every Host has. The line's argument is set as
//! options first, as `#!/bin/bash -e` asks.
//!
//! Any other interpreter is the file the line names when that file exists,
//! else the program of that name on the command's `PATH`: `#!/usr/bin/env
//! python3` runs `python3` with the rest of the line as its arguments.

use std::{
    ffi::{OsStr, OsString},
    io::{self, Read as _},
    path::Path,
};

use brush_core::execution_host::ScriptInterpreter;

/// The most of a first line that is read; Linux's own limit is 255.
const LINE_BYTES: u64 = 1024;

/// What runs `script`, from its first line; none when it has no `#!` line, or cannot be read, which its start
/// then reports. `path` is the `PATH` the command starts under.
///
/// An interpreter that is nowhere is `NotFound`, worded as bash words it:
/// `/bin/nosuch: bad interpreter: No such file or directory`.
pub fn command(
    script: &Path,
    path: Option<&OsStr>,
) -> io::Result<Option<ScriptInterpreter>> {
    let mut head = Vec::new();
    let Ok(file) = std::fs::File::open(script) else {
        return Ok(None);
    };
    if file.take(LINE_BYTES).read_to_end(&mut head).is_err() {
        return Ok(None);
    }
    let Some((interpreter, argument)) = parse(&head) else {
        return Ok(None);
    };
    // `env` names the program to run in its arguments, which Windows has no
    // `env` program to read.
    let (name, leading) = if file_stem(&interpreter) == "env" {
        let mut words = argument
            .as_deref()
            .unwrap_or_default()
            .split_whitespace()
            .skip_while(|word| *word == "-S");
        let Some(program) = words.next() else {
            return Ok(None);
        };
        (
            program.to_owned(),
            words.map(OsString::from).collect::<Vec<_>>(),
        )
    } else {
        (interpreter, argument.map(OsString::from).into_iter().collect())
    };
    if matches!(file_stem(&name), "sh" | "bash") {
        return Ok(Some(ScriptInterpreter::Shell(leading)));
    }
    let Some(program) = find(&name, path) else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{name}: bad interpreter: No such file or directory"),
        ));
    };
    Ok(Some(ScriptInterpreter::Program(program, leading)))
}

/// The interpreter a first line names and its one optional argument, the
/// rest of the line: `#!/bin/sh -e` is `/bin/sh` and `-e`. None for a file
/// that does not begin with `#!` and an interpreter.
fn parse(head: &[u8]) -> Option<(String, Option<String>)> {
    let line = head.strip_prefix(b"#!")?.split(|byte| *byte == b'\n').next()?;
    let line = String::from_utf8_lossy(line);
    let line = line.trim_matches(|c: char| c.is_whitespace());
    let (interpreter, rest) = match line.split_once(char::is_whitespace) {
        Some((interpreter, rest)) => (interpreter, rest.trim()),
        None => (line, ""),
    };
    (!interpreter.is_empty())
        .then(|| (interpreter.to_owned(), (!rest.is_empty()).then(|| rest.to_owned())))
}

/// The name of the program at `path` without its directory and its
/// extension.
fn file_stem(path: &str) -> &str {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    name.rsplit_once('.').map_or(name, |(stem, _)| stem)
}

/// The file that `name` names: itself when it exists, else the program of
/// its file name that the directories of `path` hold first, as Windows
/// names programs, with or without `.exe`.
fn find(name: &str, path: Option<&OsStr>) -> Option<std::path::PathBuf> {
    let named = Path::new(name);
    if named.is_file() {
        return Some(named.to_owned());
    }
    let file_name = name.rsplit(['/', '\\']).next().unwrap_or(name);
    std::env::split_paths(path?).find_map(|directory| {
        ["", ".exe", ".com"]
            .into_iter()
            .map(|extension| directory.join(format!("{file_name}{extension}")))
            .find(|candidate| candidate.is_file())
    })
}

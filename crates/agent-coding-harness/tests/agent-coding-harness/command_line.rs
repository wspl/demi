//! A job's command line as the runner reads it: the `demi` root pinned
//! to a package descriptor nobody checks, the command it names selected
//! and its input read, stdin body included.

use std::convert::Infallible;

use demi_command_declarations::{Node, Parsed, UsageError};
use demi_host_interface::CommandSet;

use demi_agent_coding_harness::{DemiOptions, demi_root};

/// The `demi` commands with the `browser` group, and their manifest root.
pub fn demi() -> (CommandSet, Node) {
    let mut commands = CommandSet::new();
    commands
        .register(demi_root(DemiOptions {
            browser: true,
            ..DemiOptions::default()
        }))
        .unwrap();
    let root = commands
        .declarations()
        .next()
        .unwrap()
        .pin(&mut |_| Ok::<_, Infallible>("0".repeat(64)))
        .unwrap();
    (commands, root)
}

/// The input `demi <line>` gives its command, with `stdin` as the body.
pub fn parse(root: &Node, line: &[&str], stdin: Option<&str>) -> Result<Parsed, UsageError> {
    let argv = argv(line);
    let selected = root.select(&argv)?;
    let parsed = selected.parse(&argv)?;
    match selected.node.leaf() {
        Some(leaf) if !parsed.help => parsed.validate(leaf, stdin.map(str::to_owned)),
        _ => Ok(parsed),
    }
}

/// What `demi <line> --help` prints.
pub fn help(root: &Node, line: &[&str]) -> String {
    let selected = root.select(&argv(line)).unwrap();
    selected.node.help(&selected.path.join(" "))
}

pub fn argv(line: &[&str]) -> Vec<String> {
    line.iter().map(|word| (*word).to_owned()).collect()
}

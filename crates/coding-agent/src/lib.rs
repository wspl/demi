//! The coding agent (`crates-and-packages.md` § coding-agent): the harness
//! Demi's conversations run, its system prompt, and the `demi` command root
//! its shell offers: `file` and `browser`, which run in the `demi.builtin`
//! native package, `todo`, whose handlers run in the backend over the
//! node's command storage, and the product's own groups.

mod browser;
mod demi;
mod file;
mod harness;
mod todo;

pub use demi::{DemiOptions, demi_root};
pub use harness::{CodingHarness, HostResolver};

#[cfg(test)]
mod command_line {
    //! A job's command line as the runner reads it: the `demi` root pinned
    //! to a package descriptor nobody checks, the command it names selected
    //! and its input read, stdin body included.

    use std::convert::Infallible;

    use demi_command_tree::{Node, Parsed, UsageError};
    use demi_shell::CommandSet;

    use crate::{DemiOptions, demi_root};

    /// The `demi` commands with the browser, and their manifest root.
    pub(crate) fn demi() -> (CommandSet, Node) {
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
    pub(crate) fn parse(
        root: &Node,
        line: &[&str],
        stdin: Option<&str>,
    ) -> Result<Parsed, UsageError> {
        let argv = argv(line);
        let selected = root.select(&argv)?;
        let parsed = selected.parse(&argv)?;
        match selected.node.leaf() {
            Some(leaf) if !parsed.help => parsed.validate(leaf, stdin.map(str::to_owned)),
            _ => Ok(parsed),
        }
    }

    /// What `demi <line> --help` prints.
    pub(crate) fn help(root: &Node, line: &[&str]) -> String {
        let selected = root.select(&argv(line)).unwrap();
        selected.node.help(&selected.path.join(" "))
    }

    pub(crate) fn argv(line: &[&str]) -> Vec<String> {
        line.iter().map(|word| (*word).to_owned()).collect()
    }
}

//! The `demi` root. Every Demi-specific capability lives under it, and each
//! of its subcommands is a noun domain group (file, todo, browser, agent,
//! host, …); anything outside `demi` is an ordinary shell command.

use demi_shell::GroupBuilder;

use crate::{browser::browser_group, file::file_group, todo::todo_group};

/// What the `demi` root carries besides `todo`. The native groups, `file`
/// and `browser`, are offered only where their package is published: a
/// command whose package no catalog serves fails every manifest that
/// declares it, and with it the node's shell.
pub struct DemiOptions {
    /// Whether the Host's files are offered as `demi file`.
    pub file: bool,
    /// Whether the Host's browser is offered as `demi browser`.
    pub browser: bool,
    /// The product's own groups, such as the backend's `host` group.
    pub extra: Vec<GroupBuilder>,
}

impl Default for DemiOptions {
    /// `demi file` and `demi todo`, as a catalog that serves the files
    /// offers them.
    fn default() -> Self {
        Self {
            file: true,
            browser: false,
            extra: Vec::new(),
        }
    }
}

pub fn demi_root(options: DemiOptions) -> GroupBuilder {
    let mut root = GroupBuilder::new(
        "demi",
        "The Demi platform command: every subcommand is a platform domain (file, todo, …).",
    );
    if options.file {
        root = root.group(file_group());
    }
    root = root.group(todo_group());
    if options.browser {
        root = root.group(browser_group());
    }
    options.extra.into_iter().fold(root, GroupBuilder::group)
}

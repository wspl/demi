//! The `demi` root. Every Demi-specific capability lives under it, and each
//! of its subcommands is a noun domain group (file, todo, browser, agent,
//! host, …); anything outside `demi` is an ordinary shell command.

use demi_shell::GroupBuilder;

use crate::{browser::browser_group, file::file_group, todo::todo_group};

/// What the `demi` root carries besides `file` and `todo`.
#[derive(Default)]
pub struct DemiOptions {
    /// Whether the Host's browser is offered as `demi browser`.
    pub browser: bool,
    /// The product's own groups, such as the backend's `host` group.
    pub extra: Vec<GroupBuilder>,
}

pub fn demi_root(options: DemiOptions) -> GroupBuilder {
    let mut root = GroupBuilder::new(
        "demi",
        "The Demi platform command: every subcommand is a platform domain (file, todo, …).",
    )
    .group(file_group())
    .group(todo_group());
    if options.browser {
        root = root.group(browser_group());
    }
    options.extra.into_iter().fold(root, GroupBuilder::group)
}

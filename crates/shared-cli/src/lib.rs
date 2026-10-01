//! What every program's command line shares (`crates-and-packages.md` §
//! `shared-cli`): a variable under the program's prefix that none of its
//! settings reads stops startup, so a misspelt setting is never ignored.

use std::ffi::OsString;

/// The first variable of `vars` whose name starts with `prefix` and that no
/// argument of `command` reads; a name that is not Unicode is no setting's.
pub fn unknown_variable(
    command: &clap::Command,
    prefix: &str,
    vars: impl IntoIterator<Item = (OsString, OsString)>,
) -> Option<String> {
    let known: Vec<_> = command
        .get_arguments()
        .filter_map(clap::Arg::get_env)
        .collect();
    vars.into_iter().find_map(|(name, _)| {
        let name = name.into_string().ok()?;
        let unknown =
            name.starts_with(prefix) && !known.iter().any(|known| *known == name.as_str());
        unknown.then_some(name)
    })
}

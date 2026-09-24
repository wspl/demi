//! The root command names a command set refuses (`commands.md` § Declare a
//! command): a root with one of them would shadow the shell's own words, the
//! builtins every shell provides, or a program a script expects to find.

/// Every reserved root name: shell words and builtins, the usual Unix tools,
/// and the toolchains a coding agent calls by name.
pub const RESERVED_NAMES: &[&str] = &[
    // Shell language words and builtins.
    ".", "bash", "break", "cd", "command", "continue", "echo", "exit", "export", "jobs", "local",
    "popd", "printf", "pushd", "read", "return", "set", "sh", "shift", "source", "test", "true",
    "false", "unset", "wait", // Coreutils and text tools.
    "awk", "cat", "chmod", "cp", "cut", "du", "file", "find", "grep", "head", "jq", "ls", "mkdir",
    "mv", "nl", "rg", "rm", "sed", "sort", "stat", "tail", "tee", "touch", "tr", "tree", "uniq",
    "wc", "xargs", "yq", // Toolchains.
    "bun", "cargo", "docker", "git", "go", "node", "npm", "pnpm", "python", "python3", "ruby",
    "rustc", "yarn",
];

/// Whether `name` is reserved for the shell and the system.
pub fn is_reserved(name: &str) -> bool {
    RESERVED_NAMES.contains(&name)
}

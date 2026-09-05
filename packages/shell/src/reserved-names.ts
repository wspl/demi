// Root-command names a registry refuses (`CommandRegistry`): a root with one
// of these names would shadow the shell's own words, the builtins every
// engine provides, or a system tool a script on a machine expects to find.
// One declared table, shared by every engine.

/** Shell language words and builtins the interpreters own. */
const SHELL_WORDS = ['.', 'bash', 'break', 'cd', 'command', 'continue', 'echo', 'exit', 'export', 'jobs', 'local', 'popd', 'printf', 'pushd', 'read', 'return', 'set', 'sh', 'shift', 'source', 'test', 'true', 'false', 'unset', 'wait']

/** Coreutils and text tools: tinybash's builtin set and the machine's usual companions. */
const UNIX_TOOLS = ['awk', 'cat', 'chmod', 'cp', 'cut', 'du', 'file', 'find', 'grep', 'head', 'jq', 'ls', 'mkdir', 'mv', 'nl', 'rg', 'rm', 'sed', 'sort', 'stat', 'tail', 'tee', 'touch', 'tr', 'tree', 'uniq', 'wc', 'xargs', 'yq']

/** Toolchains a coding agent invokes by name. */
const SYSTEM_TOOLS = ['bun', 'cargo', 'docker', 'git', 'go', 'node', 'npm', 'pnpm', 'python', 'python3', 'ruby', 'rustc', 'yarn']

export const RESERVED_COMMAND_NAMES: ReadonlySet<string> = new Set([...SHELL_WORDS, ...UNIX_TOOLS, ...SYSTEM_TOOLS])

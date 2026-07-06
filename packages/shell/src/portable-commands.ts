import type { CommandName } from '@demicodes/just-bash/commands'

/**
 * Fork portable commands BashEnvironment registers in every shell, so
 * cat/ls/grep-class tools work without local coreutils on any Host backend.
 */
export const DEMI_PORTABLE_COMMANDS: readonly CommandName[] = [
  'cat',
  'ls',
  'mkdir',
  'rmdir',
  'touch',
  'rm',
  'cp',
  'mv',
  'ln',
  'chmod',
  'readlink',
  'head',
  'tail',
  'wc',
  'stat',
  'grep',
  'fgrep',
  'egrep',
  'rg',
  'sed',
  'awk',
  'sort',
  'uniq',
  'comm',
  'cut',
  'paste',
  'tr',
  'rev',
  'nl',
  'fold',
  'expand',
  'unexpand',
  'strings',
  'column',
  'join',
  'tee',
  'find',
  'basename',
  'dirname',
  'tree',
  'du',
  'jq',
  'base64',
  'diff',
  'seq',
  'expr',
  'md5sum',
  'sha1sum',
  'sha256sum',
  'file',
  'tac',
  'od',
]

/** Shell language words and builtins the interpreter itself owns. */
const SHELL_BUILTIN_NAMES = [
  '.',
  'bash',
  'break',
  'cd',
  'command',
  'continue',
  'echo',
  'exit',
  'export',
  'jobs',
  'local',
  'popd',
  'printf',
  'pushd',
  'read',
  'return',
  'set',
  'sh',
  'shift',
  'source',
  'test',
  'unset',
  'wait',
]

/** Ecosystem tools the model expects to reach through Host.process.spawn. */
const SYSTEM_TOOL_NAMES = [
  'bun',
  'cargo',
  'docker',
  'git',
  'go',
  'node',
  'npm',
  'pnpm',
  'python',
  'python3',
  'ruby',
  'rustc',
  'xargs',
  'yarn',
  'yq',
]

/**
 * Names registered commands must not shadow, derived from the actual portable
 * command set plus interpreter builtins and pass-through system tools — not a
 * hand-maintained parallel list.
 */
export const RESERVED_COMMAND_NAMES: ReadonlySet<string> = new Set([
  ...DEMI_PORTABLE_COMMANDS,
  ...SHELL_BUILTIN_NAMES,
  ...SYSTEM_TOOL_NAMES,
])

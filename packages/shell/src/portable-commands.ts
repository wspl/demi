import { getCommandNames, type CommandName } from '@demicodes/just-bash/commands'

/**
 * Deliberately routed to real OS processes (Host.process.spawn) even though
 * just-bash ships portable versions. This is a semantics decision, not a
 * missing feature:
 *
 * - `bash`, `sh`: scripts in real repositories expect a real interpreter
 *   (full bash semantics, real coreutils, real subprocess behavior). Routing
 *   them into just-bash's simulated interpreter would silently downgrade
 *   every `bash script.sh` a model runs. (The fork's nested-interpreter
 *   stdout also does not forward through this package's wrapping — but even
 *   with that fixed, real spawn stays the right routing.)
 * - `sleep`: must be a real, abortable OS process so foreground abort and
 *   timeout semantics (`env.abort()` while `running`) genuinely interrupt
 *   it; just-bash's version is a plain in-process timer.
 */
const REAL_SPAWN_DEPENDENT_COMMANDS = new Set(['bash', 'sh', 'sleep'])

/**
 * just-bash portable names that implement bash builtins (or shell state).
 * They stay in-process. `true`/`false` are omitted from the registry so the
 * interpreter builtins run.
 */
const IN_PROCESS_PORTABLE_COMMANDS = new Set([
  'echo',
  'printf',
  'pwd',
  'alias',
  'unalias',
  'history',
  'help',
  'time',
])

/**
 * Fork portable commands BashEnvironment registers in every shell, so
 * cat/ls/grep-class tools work without local coreutils on any Host backend.
 *
 * Sourced directly from just-bash's own registry instead of a hand-maintained
 * copy: `CommandName` is just-bash's own "safe to run anywhere" set — it
 * already excludes the commands that need real process/network access
 * (curl → NetworkCommandName, python3/python → PythonCommandName, js-exec/node
 * → JavaScriptCommandName all live in separate, non-portable type unions).
 * Unix names `hostSpawn` first (`preferHostSpawn`); portable is fallback for
 * `executable_not_found` only.
 */
export const DEMI_PORTABLE_COMMANDS: readonly CommandName[] = getCommandNames().filter(
  (name) => !REAL_SPAWN_DEPENDENT_COMMANDS.has(name) && name !== 'true' && name !== 'false',
) as CommandName[]

export function shouldPreferHostSpawn(name: string): boolean {
  return !IN_PROCESS_PORTABLE_COMMANDS.has(name)
}

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
  'true',
  'false',
  'unset',
  'wait',
]

/**
 * Ecosystem tools the model expects to reach through Host.process.spawn.
 *
 * xargs and yq stay out of this list: they are already in the portable set
 * (`preferHostSpawn`, then the in-process implementation if the Host has no
 * binary). Listing them here would duplicate a reserved name.
 */
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
  'yarn',
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

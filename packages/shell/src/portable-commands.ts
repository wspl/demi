import { getCommandNames, type CommandName } from '@demicodes/just-bash/commands'

/**
 * Individually excluded even though just-bash calls them portable: each
 * regresses a real, already-passing test elsewhere in this monorepo when
 * dispatched through this package's fork-command wrapping specifically
 * (isolated just-bash unit tests for these same commands pass fine, so the
 * break is in how @demicodes/shell wraps them, not in just-bash itself):
 *
 * - `bash`, `sh`: nested "run a script string through the interpreter again".
 *   Their stdout never reaches the caller here — `sh -c "printf hi"` and
 *   `bash -c "echo hi"` both exit 0 with empty output, while the exact same
 *   scripts run un-nested work fine. This package's own `AgentServer` test
 *   spawns `sh -c` for real and asserts on the real output.
 * - `sleep`: just-bash's version is a plain timer, not a real backgroundable,
 *   abortable OS process. This package's own environment tests use `sleep 10`
 *   specifically as a long-running foreground process to exercise abort/
 *   timeout handling (`env.abort()` / status-while-`running`), which needs a
 *   real process to abort.
 *
 * Keeping these three off the list preserves their pre-existing
 * fall-through to Host.process.spawn, same as before this list started
 * tracking just-bash's registry.
 */
const REAL_SPAWN_DEPENDENT_COMMANDS = new Set(['bash', 'sh', 'sleep'])

/**
 * Fork portable commands BashEnvironment registers in every shell, so
 * cat/ls/grep-class tools work without local coreutils on any Host backend.
 *
 * Sourced directly from just-bash's own registry instead of a hand-maintained
 * copy: `CommandName` is just-bash's own "safe to run anywhere" set — it
 * already excludes the commands that need real process/network access
 * (curl → NetworkCommandName, python3/python → PythonCommandName, js-exec/node
 * → JavaScriptCommandName all live in separate, non-portable type unions). A
 * hand-picked subset of this list drifts silently as just-bash adds commands
 * (this one was missing echo, pwd, printf, date, env, hostname, whoami, gzip,
 * tar, split, sqlite3, xan, and more — all already categorized as safe
 * upstream, just never backported here). Deriving it removes that drift
 * entirely: everything just-bash calls portable (minus the exceptions above),
 * we call portable.
 */
export const DEMI_PORTABLE_COMMANDS: readonly CommandName[] = getCommandNames().filter(
  (name) => !REAL_SPAWN_DEPENDENT_COMMANDS.has(name),
) as CommandName[]

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

/**
 * Ecosystem tools the model expects to reach through Host.process.spawn.
 *
 * xargs and yq are deliberately absent even though real versions exist: both
 * now have a real (non-simulated-elsewhere) implementation in just-bash's own
 * portable set above, which registers and dispatches before Host.process.spawn
 * is ever consulted — listing them here too would claim they fall through to
 * a real spawn, which no longer happens.
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

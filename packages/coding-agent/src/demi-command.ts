import type { CommandGroup } from '@demicodes/shell'
import { createFileGroup } from './commands/file/group'
import { createTodoCommand } from './todo-command'

export interface DemiCommandOptions {
  /** Product-contributed subcommand groups (e.g. the backend's `host` group). */
  extraSubcommands?: CommandGroup[]
}

/**
 * The `demi` root. Organizing rule: every Demi-specific capability lives
 * under `demi`, and every `demi` subcommand is a noun domain group (file,
 * todo, agent, host, …); anything outside `demi` is an ordinary shell
 * command.
 */
export function createDemiCommand(options: DemiCommandOptions = {}): CommandGroup {
  return {
    name: 'demi',
    summary: 'The Demi platform command: every subcommand is a platform domain (file, todo, …).',
    subcommands: [createFileGroup(), createTodoCommand(), ...(options.extraSubcommands ?? [])],
  }
}

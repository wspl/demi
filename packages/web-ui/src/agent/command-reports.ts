import type { CommandReport } from '@demicodes/protocol'
import { inject, provide, type InjectionKey } from 'vue'
import type { CommandEndMark } from './command-end'

/**
 * The call a report row names (`runtime.md` § Command reports): its title, or
 * the command by its number when the title is not known.
 */
export function reportTitle(report: CommandReport): string {
  return report.title.trim() || `Command ${report.commandId}`
}

/**
 * What happened, in the user's words, which a report row sets apart from the
 * title: a title is an imperative, so the two never read as one sentence.
 */
export function reportOutcome(report: CommandReport): string {
  const event = report.event
  switch (event.kind) {
    case 'running':
      return 'still running'
    case 'ended': {
      // An end whose status was not recorded reads as one that succeeded.
      const code = event.exitCode ?? 0
      return code === 0 ? 'ended' : `ended with exit code ${code}`
    }
    case 'stopped':
      switch (event.by?.kind) {
        case 'user':
          return 'stopped by you'
        case 'agent':
          return 'stopped by another agent'
        default:
          return 'stopped'
      }
    case 'lost':
      return 'lost'
  }
}

/** The tag a report row carries, as a shell row marks its command's end: only what went wrong. */
export function reportMark(report: CommandReport): CommandEndMark | null {
  const event = report.event
  switch (event.kind) {
    case 'running':
      return null
    case 'ended': {
      const code = event.exitCode ?? 0
      return code === 0 ? null : { kind: 'failed', exitCode: code }
    }
    case 'stopped':
      return { kind: 'stopped' }
    case 'lost':
      return { kind: 'lost', reason: event.reason }
  }
}

/**
 * Opens a command's terminal tab, as a click on a row that names the command
 * does; none for a command no terminal tab can show.
 */
export type CommandOpener = (commandId: string) => (() => void) | undefined

const commandOpenerKey: InjectionKey<CommandOpener> = Symbol('command-opener')

/** Gives the rows below the way to open a command's terminal tab. */
export function provideCommandOpener(opener: CommandOpener): void {
  provide(commandOpenerKey, opener)
}

/** The way to open a command's terminal tab; a list shown alone opens none. */
export function useCommandOpener(): CommandOpener {
  return inject(commandOpenerKey, () => undefined)
}

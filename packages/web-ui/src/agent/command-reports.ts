import type { CommandReport } from '@demicodes/protocol'
import { inject, provide, type InjectionKey } from 'vue'
import type { CommandEndMark } from './command-end'

/**
 * What a report row reads (`runtime.md` § Command reports): the call's title
 * and what happened, in the user's words. A report of a call whose title is
 * not known names the command by its number.
 */
export function reportSentence(report: CommandReport): string {
  const title = report.title.trim() || `Command ${report.commandId}`
  const event = report.event
  switch (event.kind) {
    case 'running':
      return `${title} is still running`
    case 'ended': {
      // An end whose status was not recorded reads as one that succeeded.
      const code = event.exitCode ?? 0
      return code === 0 ? `${title} ended` : `${title} ended with exit code ${code}`
    }
    case 'stopped':
      switch (event.by?.kind) {
        case 'user':
          return `${title} was stopped by you`
        case 'agent':
          return `${title} was stopped by another agent`
        default:
          return `${title} was stopped`
      }
    case 'lost':
      return `${title} was lost`
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

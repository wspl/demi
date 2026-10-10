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
 * What a report row says happened, in the user's words, before the title it
 * names: a title is an imperative, so it stands as the object of the notice,
 * never as its subject. A report of progress has no row.
 */
export function reportNotice(report: CommandReport): string | null {
  const event = report.event
  switch (event.kind) {
    case 'running':
      return null
    case 'ended': {
      // An end whose status was not recorded reads as one that succeeded.
      const code = event.exitCode ?? 0
      return code === 0 ? 'Command finished' : `Command failed with exit code ${code}`
    }
    case 'stopped':
      switch (event.by?.kind) {
        case 'user':
          return 'Command stopped by you'
        case 'agent':
          return 'Command stopped by another agent'
        default:
          return 'Command stopped'
      }
    case 'lost':
      return 'Command lost'
  }
}

/** Whether a report shows as a row: its command's end does, its progress does not. */
export function reportShown(report: CommandReport): boolean {
  return report.event.kind !== 'running'
}

/**
 * Brings the call that started a command into view and marks it, as a
 * report row's title asks: from the window shown when it holds the call,
 * else by the host, which reads where the call lies; none where neither can.
 */
export type CommandRevealer = (commandId: string) => (() => void) | undefined

const commandRevealerKey: InjectionKey<CommandRevealer> = Symbol('command-revealer')

/** Gives the rows below the way to bring a command's call into view. */
export function provideCommandRevealer(revealer: CommandRevealer): void {
  provide(commandRevealerKey, revealer)
}

/** The way to bring a command's call into view; a block shown alone reaches none. */
export function useCommandRevealer(): CommandRevealer {
  return inject(commandRevealerKey, () => undefined)
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

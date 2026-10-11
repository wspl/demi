import type { ShellViewStatus } from '@demicodes/protocol'

/**
 * Where a command is, as a stored view or the command's live frames say it:
 * running, exited with its code, stopped (`aborted`), or lost with why.
 */
export interface CommandEnd {
  status: ShellViewStatus
  exitCode?: number
  lostReason?: string
}

/**
 * How a command ended, as a shell row or a report row marks it (`runtime.md`
 * § Rendering boundary, § Command reports): a row marks only what went
 * wrong, and a call's row and its notice mark it alike.
 */
export type CommandEndMark =
  | { kind: 'failed', exitCode: number }
  | { kind: 'stopped' }
  | { kind: 'lost', reason: string }

/** The mark of the command `view` shows; none while it runs or once it succeeded. */
export function commandEndMark(view: CommandEnd | null): CommandEndMark | null {
  switch (view?.status) {
    case 'exited':
      return view.exitCode !== undefined && view.exitCode !== 0
        ? { kind: 'failed', exitCode: view.exitCode }
        : null
    case 'aborted':
      return { kind: 'stopped' }
    case 'lost':
      return { kind: 'lost', reason: view.lostReason ?? '' }
    default:
      return null
  }
}

/**
 * The end of the command `view` shows, in words, as an open row says it
 * above the output: only when it failed, was stopped or was lost, as the
 * row's tag.
 */
export function commandEndWords(view: CommandEnd | null): string | null {
  const mark = commandEndMark(view)
  switch (mark?.kind) {
    case 'failed':
      return `Exited with code ${mark.exitCode}`
    case 'stopped':
      return 'Stopped'
    case 'lost':
      return mark.reason ? `Lost: ${mark.reason}` : 'Lost'
    default:
      return null
  }
}

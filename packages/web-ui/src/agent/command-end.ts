import type { ShellViewStatus } from '@demicodes/protocol'
import { formatDuration } from './thinking-label'

/**
 * Where a command is, as a stored view or the command's live frames say it:
 * running, and whether a stop waits for its end, exited with its code,
 * stopped (`aborted`), or lost with why.
 */
export interface CommandEnd {
  status: ShellViewStatus
  exitCode?: number
  lostReason?: string
  stopping?: boolean
}

/**
 * How a command ended, as a shell row or a report row marks it (`runtime.md`
 * § Rendering boundary, § Command reports), or that a stop waits for its
 * runner to end it (`runner.md` § Cancellation and completion): a row marks
 * only what went wrong, and a call's row and its notice mark it alike.
 */
export type CommandEndMark =
  | { kind: 'failed', exitCode: number }
  | { kind: 'stopping' }
  | { kind: 'stopped' }
  | { kind: 'lost', reason: string }

/** The mark of the command `view` shows; none while it runs unstopped or once it succeeded. */
export function commandEndMark(view: CommandEnd | null): CommandEndMark | null {
  switch (view?.status) {
    case 'running':
      return view.stopping ? { kind: 'stopping' } : null
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
 * How long a command runs before its row wears the time as a tag
 * (`runtime.md` § Rendering boundary), and a run before its title says it
 * (§ Work groups): a quicker one shows none, so it does not flash one.
 */
export const SHOW_TIME_AFTER_MS = 3000

/** How long work has run, as a running row says it once it has run more than 3 seconds; none before. */
export function runningTime(elapsedMs: number | null): string | null {
  return elapsedMs !== null && elapsedMs > SHOW_TIME_AFTER_MS ? formatDuration(elapsedMs) : null
}

/**
 * What an open shell row says above the output (`runtime.md` § Rendering
 * boundary): how long the command has run, while it runs and `end` is
 * none, or how it ended and after how long, given how long it ran or has
 * run, `elapsedMs`. A lost command says why, as its tag does, and no time;
 * one a stop waits for says so, as its tag does (`runner.md`
 * § Cancellation and completion); none while the time is unknown.
 */
export function commandStatusLine(end: CommandEnd | null, elapsedMs: number | null): string | null {
  if (end?.status === 'lost') {
    return end.lostReason ? `Lost: ${end.lostReason}` : 'Lost'
  }
  if (end?.stopping) {
    return 'Stopping…'
  }
  if (elapsedMs === null) {
    return null
  }
  const time = formatDuration(elapsedMs)
  switch (end?.status) {
    case undefined:
    case 'running':
      return `Running for ${time}`
    case 'exited':
      if (end.exitCode === undefined)
        return `Exited after ${time}`
      return end.exitCode === 0 ? `Exited with 0 after ${time}` : `Failed with exit code ${end.exitCode} after ${time}`
    case 'aborted':
      return `Stopped after ${time}`
  }
}

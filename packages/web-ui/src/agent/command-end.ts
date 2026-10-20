import type { ShellViewStatus } from '@demicodes/protocol'
import { formatSpan } from '../ui/dates'
import { t } from './locales/text'

/**
 * Where a command is, as a stored view or the command's live frames say it:
 * running, and whether a stop waits for its end or its start for its Host,
 * the device `waitingFor` names, exited with its code, stopped (`aborted`),
 * or lost with why.
 */
export interface CommandEnd {
  status: ShellViewStatus
  exitCode?: number
  lostReason?: string
  stopping?: boolean
  waitingFor?: string
}

/**
 * How a command ended, as a shell row or a report row marks it (`rendering.md`
 * § Rendering boundary, § Command reports), that a stop waits for its
 * runner to end it (`crates/runner-shell/src/job.rs`), or that its
 * start waits for its Host, an offline device (`crates/backend-host-access/src/access.rs`): a row marks only what is not going as planned, and
 * a call's row and its notice mark it alike.
 */
export type CommandEndMark =
  | { kind: 'failed', exitCode: number }
  | { kind: 'stopping' }
  | { kind: 'waiting', host: string }
  | { kind: 'stopped' }
  | { kind: 'lost', reason: string }

/** The mark of the command `view` shows; none while it runs unstopped or once it succeeded. */
export function commandEndMark(view: CommandEnd | null): CommandEndMark | null {
  switch (view?.status) {
    case 'running':
      if (view.stopping) {
        return { kind: 'stopping' }
      }
      return view.waitingFor === undefined ? null : { kind: 'waiting', host: view.waitingFor }
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
 * (`rendering.md` § Rendering boundary), and a run before its title says it
 * (§ Work groups): a quicker one shows none, so it does not flash one.
 */
export const SHOW_TIME_AFTER_MS = 3000

/** How long work has run, as a running row says it once it has run more than 3 seconds; none before. */
export function runningTime(elapsedMs: number | null): string | null {
  return elapsedMs !== null && elapsedMs > SHOW_TIME_AFTER_MS ? formatSpan(elapsedMs) : null
}

/**
 * What an open shell row says above the output (`rendering.md` § Rendering
 * boundary): how long the command has run, while it runs and `end` is
 * none, or how it ended and after how long, given how long it ran or has
 * run, `elapsedMs`. A lost command says why, as its tag does, and no time;
 * one a stop waits for says so, as its tag does (`crates/runner-shell/src/job.rs`), and so does one whose start waits for
 * its Host (`crates/backend-host-access/src/access.rs`); none while the
 * time is unknown.
 */
export function commandStatusLine(end: CommandEnd | null, elapsedMs: number | null): string | null {
  if (end?.status === 'lost') {
    return end.lostReason ? t('transcript.commandStatus.lostWhy', { reason: end.lostReason }) : t('transcript.commandStatus.lost')
  }
  if (end?.stopping) {
    return t('transcript.commandStatus.stopping')
  }
  if (end?.waitingFor !== undefined) {
    return t('transcript.commandStatus.waiting', { host: end.waitingFor })
  }
  if (elapsedMs === null) {
    return null
  }
  const span = formatSpan(elapsedMs)
  switch (end?.status) {
    case undefined:
    case 'running':
      return t('transcript.commandStatus.running', { span })
    case 'exited':
      if (end.exitCode === undefined)
        return t('transcript.commandStatus.exited', { span })
      return end.exitCode === 0
        ? t('transcript.commandStatus.exitedZero', { span })
        : t('transcript.commandStatus.failed', { code: end.exitCode, span })
    case 'aborted':
      return t('transcript.commandStatus.stopped', { span })
  }
}

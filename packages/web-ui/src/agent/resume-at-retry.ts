import { inject, provide, type InjectionKey } from 'vue'
import { formatRetryAt } from '../ui/dates'

/**
 * The user's preference to resume a turn by itself at the time its failure
 * names, such as a usage limit that resets at 9:00 AM (`turns.md`
 * § Recovering an unfinished turn): what it is now, and how a control
 * changes it. One preference, shown in Settings › General and on each such
 * failure's record.
 */
export interface ResumeAtRetry {
  on: boolean
  set(on: boolean): void
}

const resumeAtRetryKey: InjectionKey<() => ResumeAtRetry | undefined> = Symbol('resume-at-retry')

/**
 * Offers the preference to the failure records below, read at each render;
 * none where the host does not offer it, and the records then carry no
 * control.
 */
export function provideResumeAtRetry(source: () => ResumeAtRetry | undefined): void {
  provide(resumeAtRetryKey, source)
}

export function useResumeAtRetry(): () => ResumeAtRetry | undefined {
  return inject(resumeAtRetryKey, () => undefined)
}

/**
 * The label of the Resume at control a failure's record carries, or none: a
 * record of the conversation's own agent, where the host offers the
 * preference, while the time its failure names is still to come at `now`,
 * in milliseconds. A subagent's turn is never resumed by itself, since its
 * parent decides for it, and a time that has passed leaves nothing to
 * resume at.
 */
export function resumeAtLabel(
  retryAt: string | null,
  ownAgent: boolean,
  offered: boolean,
  now: number,
): string | null {
  if (!offered || !ownAgent || retryAt === null || Date.parse(retryAt) <= now) {
    return null
  }
  return formatRetryAt('Resume', retryAt, new Date(now))
}

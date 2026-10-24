import { inject, provide, type InjectionKey } from 'vue'
import type { ScheduleMark } from '@demicodes/protocol'
import { shortMoment } from '../schedules/words'

/**
 * What sent a user message that the user did not write: a schedule, named in
 * the transcript above the message as a receipt names the agent that sent it,
 * with when it fired, and the name opening the conversation's Schedules menu.
 */
export interface MessageOrigin {
  /** The schedule's name when it fired. */
  name: string
  /** When it fired, in words: Oct 17 09:00. */
  when: string
  /** Opens the Schedules menu; none where nothing can, or once the schedule is gone. */
  go?: () => void
}

/**
 * What opens the conversation's Schedules menu from a mark in the transcript:
 * for the schedule `id`, a way to open it while the conversation still has
 * that schedule, none once it is gone.
 */
export type ScheduleOpener = (id: string) => (() => void) | undefined

const scheduleOpenerKey: InjectionKey<ScheduleOpener> = Symbol('schedule-opener')

/** Gives the transcript below a way to open the Schedules menu from the schedule a message names. */
export function provideScheduleOpener(open: ScheduleOpener): void {
  provide(scheduleOpenerKey, open)
}

/**
 * The origin a message's schedule mark says, leading to the Schedules menu
 * where the page gave a way to open it and the schedule is still there; null
 * for a message the user wrote. It reads the schedules as it is called, so a
 * render that calls it follows them.
 */
export function useMessageOrigin(): (mark: ScheduleMark | null | undefined) => MessageOrigin | null {
  const open = inject(scheduleOpenerKey, null)
  return (mark) => mark
    ? { name: mark.name, when: shortMoment(mark.at), go: open?.(mark.scheduleId) }
    : null
}

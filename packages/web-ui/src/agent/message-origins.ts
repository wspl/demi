import { inject, provide, type InjectionKey } from 'vue'
import type { AutomationMark } from '@demicodes/protocol'
import { shortMoment } from '../automations/words'

/**
 * What sent a user message that the user did not write: an automation, named in
 * the transcript above the message as a receipt names the agent that sent it,
 * with when it fired, and the name leading to the automation.
 */
export interface MessageOrigin {
  /** The automation's name when it fired. */
  name: string
  /** When it fired, in words: Oct 17 09:00. */
  when: string
  /** Opens the automation; none where nothing can. */
  go?: () => void
}

/** Opens the automation of a mark, from the transcript. */
export type AutomationOpener = (automationId: string) => void

const automationOpenerKey: InjectionKey<AutomationOpener> = Symbol('automation-opener')

/** Gives the transcript below a way to open the automation a message names. */
export function provideAutomationOpener(open: AutomationOpener): void {
  provide(automationOpenerKey, open)
}

/**
 * The origin a message's automation mark says, leading to the automation
 * where the page gave a way to open it; null for a message the user wrote.
 */
export function useMessageOrigin(): (mark: AutomationMark | null | undefined) => MessageOrigin | null {
  const open = inject(automationOpenerKey, null)
  return (mark) => mark
    ? { name: mark.name, when: shortMoment(mark.at), go: open ? () => open(mark.automationId) : undefined }
    : null
}

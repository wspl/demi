import { isMacOS } from '@tiptap/core'
import { watch } from 'vue'
import type { SessionPhase } from '@demicodes/protocol'
import { t } from '../locales/text'
import type { TitleText } from '../ui/ui-text'

/**
 * What a message the user sends while the agent works does
 * (`product.md` § Steer or queue): it steers the running turn, which reads it
 * as soon as the call it waits on returns, or waits in the queue for the turn
 * to end. The user's preference chooses what Enter does; Steer when unset.
 */
export type SendWhileRunning = 'steer' | 'queue'

export const DEFAULT_SEND_WHILE_RUNNING: SendWhileRunning = 'steer'

/** How the composer sends a message: at once while nothing runs, otherwise one of the two ways. */
export type SendWay = 'send' | SendWhileRunning

/** A way's name, as the send button and the setting say it. */
export function sendWayLabel(way: SendWhileRunning): TitleText {
  return t(`sendWay.${way}`)
}

export function otherWay(way: SendWhileRunning): SendWhileRunning {
  return way === 'steer' ? 'queue' : 'steer'
}

/**
 * How a message goes: at once while the agent does not work; otherwise the
 * preferred way, or the other one when the user asked for it with
 * ⌘/Ctrl+Enter outside a code block or a ⌘/Ctrl-click on the send button.
 */
export function chooseSendWay(working: boolean, preferred: SendWhileRunning, asksOtherWay: boolean): SendWay {
  if (!working) {
    return 'send'
  }
  return asksOtherWay ? otherWay(preferred) : preferred
}

/** A message the user sent that the session has not confirmed: how it was sent, and why its delivery failed, null while it is on its way. */
export interface MessageOnItsWay {
  way: SendWay
  error: string | null
}

/**
 * The message on its way that starts a turn, or undefined while none does:
 * the one reading for the composer's Stop and its way of sending, the
 * transcript's Requesting row, Stop's wait and Retry's way. Its turn is as
 * good as running from the send (`product.md` § Recovering an unfinished
 * turn), so the composer shows Stop and a message sent meanwhile steers it
 * or waits in the queue (§ Steer or queue).
 *
 * It is the first message on its way that was sent as the next message,
 * which the session's phase leaves the starting one through its delivery
 * and confirmation, since the backend starts the turn before it writes the
 * message. While nothing runs, it is the first message on its way, whichever
 * way it was sent: the session answers any message next then, a steer it
 * refuses because its turn ended included, which goes as a message, and a
 * queued message whose turn ended. A message whose delivery failed starts
 * nothing.
 */
export function turnStartingMessage<T extends MessageOnItsWay>(phase: SessionPhase, sent: readonly T[]): T | undefined {
  return sent.find((message) => message.error === null && (message.way === 'send' || phase === 'idle'))
}

/**
 * Whether the agent works as far as a message sent now is concerned: a
 * turn or a compaction runs, or a message on its way starts a turn, so the
 * message steers or waits in the queue rather than going as the next one.
 */
export function agentWorks(phase: SessionPhase, sent: readonly MessageOnItsWay[]): boolean {
  return phase !== 'idle' || turnStartingMessage(phase, sent) !== undefined
}

/**
 * The key that sends a message the other way, ⌘⏎ on macOS and Ctrl+Enter
 * elsewhere: the composer's keymap reads Mod-Enter, which tiptap binds to ⌘
 * on macOS and to Ctrl elsewhere by the same test.
 */
export function otherWayKeys(): string {
  return isMacOS() ? '⌘⏎' : 'Ctrl+Enter'
}

/**
 * Enter on a message that cannot go only because what it needs is still
 * loading, as the model list is right after the page loads
 * (`product.md` § Writing a message): the send button says why, and the
 * message goes by itself, the way Enter asked, once it can. What loaded and
 * still cannot send, such as no usable model, ends the wait, and the button
 * says why then. Call it in a component's setup, whose scope ends the watch.
 */
export function useSendOnceLoaded(options: {
  /** Whether the message cannot go now. */
  blocked: () => boolean
  /** Whether what it waits for is still loading. */
  loading: () => boolean
  /** Sends the message, the other way when Enter asked for it. */
  send: (asksOtherWay: boolean) => void
}): {
  /** Keeps Enter pressed on a message that cannot go yet, while what it waits for loads; answers whether it waits. */
  keep: (asksOtherWay: boolean) => boolean
} {
  /** The Enter that waits, by whether it asked for the other way; null while none does. */
  let waiting: boolean | null = null
  watch(options.blocked, (blocked) => {
    const asksOtherWay = waiting
    if (blocked || asksOtherWay === null) {
      return
    }
    waiting = null
    options.send(asksOtherWay)
  })
  watch(options.loading, (loading) => {
    if (!loading && options.blocked()) {
      waiting = null
    }
  })
  return {
    keep: (asksOtherWay) => {
      if (!options.loading()) {
        return false
      }
      waiting = asksOtherWay
      return true
    },
  }
}

/** Whether a click on the send button asks for the other way: ⌘-click on macOS, Ctrl-click elsewhere, as the key does. */
export function clickAsksOtherWay(event: MouseEvent): boolean {
  return isMacOS() ? event.metaKey : event.ctrlKey
}

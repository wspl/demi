import { isMacOS } from '@tiptap/core'
import { watch } from 'vue'
import type { SessionPhase } from '@demicodes/protocol'
import { t } from '../locales/text'
import type { TitleText } from '../ui/ui-text'

/**
 * What a message the user sends while the agent works does: it steers the
 * running turn, which reads it as soon as the call it waits on returns, or
 * waits in the queue for the turn to end. The user's preference chooses what
 * Enter does; Steer when unset.
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
 * A message the user sent, with the ids of the messages sent after it, in
 * the order they were sent. A message not delivered keeps its place by
 * them: before the first of them that shows.
 */
export interface SentInOrder {
  id: string
  followers: string[]
}

/**
 * The message `id` is sent now: every other message held follows it with
 * `id`, but one that `settled` says no longer needs more, as one a message
 * after which the transcript holds already stands before for good.
 */
export function markSent(sent: readonly SentInOrder[], id: string, settled: (message: SentInOrder) => boolean = () => false): void {
  for (const message of sent) {
    if (message.id !== id && !message.followers.includes(id) && !settled(message)) {
      message.followers.push(id)
    }
  }
}

/**
 * The message `id` leaves the order without being delivered, or without
 * staying where the session held it: Edit or Delete takes it away, Recall
 * takes it back once its removal is done, or Retry makes it a new message
 * at the end. The messages sent before it keep their place by the ones sent
 * after it.
 */
export function markWithdrawn(sent: readonly SentInOrder[], id: string): void {
  for (const message of sent) {
    const at = message.followers.indexOf(id)
    if (at !== -1) {
      message.followers.splice(at, 1)
    }
  }
}

/**
 * Whether the dock offers Retry: a message was not delivered, and sending it
 * again can deliver it.
 */
export function offersRetry(sent: readonly (MessageOnItsWay & { retryable?: false })[]): boolean {
  return sent.some((message) => message.error !== null && message.retryable !== false)
}

/**
 * The message on its way that starts a turn, or undefined while none does:
 * the one reading for the composer's Stop and its way of sending, the
 * transcript's Requesting row, Stop's wait and Retry's way. Its turn is as
 * good as running from the send, so the composer shows Stop and a message sent
 * meanwhile steers it or waits in the queue.
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
 * loading, as the model list is right after the page loads: the send button
 * says why, and the message goes by itself, the way Enter asked, once it can.
 * What loaded and still cannot send, such as no usable model, ends the wait,
 * and the button says why then. Call it in a component's setup, whose scope
 * ends the watch.
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

import type { AgentMessage } from '@demicodes/protocol'
import { inject, provide, type InjectionKey } from 'vue'
import { homePath } from '../files/paths'

/**
 * A receipt's row (`subagents.md` § Product rendering): what happened, in
 * the user's words, and the agent it names by its description, which is
 * an imperative title and so the notice's object, never its subject.
 */
export interface ReceiptRow {
  notice: string
  /** The agent the row names; none for a row the user's own decision or Demi makes. */
  title: string | null
  /** The tag after the title: only what went wrong. */
  tag: 'failed' | 'stopped' | null
}

/** The row an agent message shows as in the transcript. */
export function receiptRow(message: AgentMessage): ReceiptRow {
  const event = message.event
  const plain = (notice: string): ReceiptRow => ({ notice, title: null, tag: null })
  switch (event.type) {
    // The user's decision on a permission request, which reached the agent as this message.
    case 'permission':
      return plain(event.outcome === 'allowed'
        ? `You allowed this conversation to ${event.action}`
        : `You denied this conversation permission to ${event.action}`)
    // The user's move of the conversation, which the user told the agent of.
    case 'moved':
      return plain(`You moved this conversation to ${event.host} · ${homePath(event.path, event.home)}`)
    // Demi's notice that a move the agent asked for failed; the message says why.
    case 'move_failed':
      return plain('Move failed')
    case 'message':
      return { notice: 'Agent sent an update', title: senderName(message), tag: null }
    case 'completion':
      switch (event.outcome) {
        case 'completed':
          return { notice: 'Agent finished', title: senderName(message), tag: null }
        case 'failed':
          return { notice: 'Agent failed', title: senderName(message), tag: 'failed' }
        case 'aborted':
          return { notice: 'Agent stopped', title: senderName(message), tag: 'stopped' }
      }
  }
}

/** The agent a message names: its description, or its number when it has none. */
function senderName(message: AgentMessage): string {
  const sender = message.sender
  return sender?.description.trim() || `Agent ${sender?.number ?? ''}`.trim()
}

/**
 * Where a receipt came from: the sender's transcript, null for the
 * conversation's own (the root, agent 0), and the block of it the receipt
 * names, if any: an update's call that sent it, a completion's last answer.
 */
export interface ReceiptOrigin {
  node: string | null
  blockId: string | null
}

/** Where `message` came from; none for a message no agent sent. */
export function receiptOrigin(message: AgentMessage): ReceiptOrigin | null {
  const sender = message.sender
  if (!sender) {
    return null
  }
  return { node: sender.number === 0 ? null : sender.id, blockId: message.origin ?? null }
}

/**
 * Opens where a receipt came from, as a click on its agent's name asks: the
 * Agents window on the sender's tab, and the block it names brought into
 * view and marked, from the window shown when it holds the block, else by
 * the host, which reads where it lies; none where nothing can.
 */
export type OriginRevealer = (origin: ReceiptOrigin) => (() => void) | undefined

const originRevealerKey: InjectionKey<OriginRevealer> = Symbol('origin-revealer')

/** Gives the receipts below the way to open where they came from. */
export function provideOriginRevealer(revealer: OriginRevealer): void {
  provide(originRevealerKey, revealer)
}

/** The way to open where a receipt came from; a block shown alone reaches none. */
export function useOriginRevealer(): OriginRevealer {
  return inject(originRevealerKey, () => undefined)
}

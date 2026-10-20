import type { AgentMessage } from '@demicodes/protocol'
import { inject, provide, type InjectionKey } from 'vue'
import { homePath } from '../files/paths'
import { codeText } from '../infra/errors'
import { hostAction, reachAction, useCredentialAction, wordedActions } from '../permissions/types'
import { around, t } from './locales/text'

/**
 * A receipt's row: what happened, in
 * the user's words, and the agent it names by its description, which is
 * an imperative title and so the notice's object, never its subject. The
 * main agent, which has no description of its own, is named as such and
 * ends the sentence without a colon: *Message from main agent*.
 */
export interface ReceiptRow {
  notice: string
  /** The agent the row names; none for a row the user's own decision or Demi makes. */
  title: string | null
  /** The title stands inside the notice's sentence, with no colon: the sentence is `notice`, the title, then `after`. */
  inline: boolean
  /** What follows an inline title in the sentence; empty where the language ends it with the title. */
  after: string
  /** The tag after the title: only what went wrong. */
  tag: 'failed' | 'stopped' | null
}

/** The row an agent message shows as in the transcript. */
export function receiptRow(message: AgentMessage): ReceiptRow {
  const event = message.event
  const plain = (notice: string): ReceiptRow => ({ notice, title: null, inline: false, after: '', tag: null })
  const named = (notice: string, tag: ReceiptRow['tag'] = null): ReceiptRow =>
    ({ notice, title: senderName(message), inline: false, after: '', tag })
  switch (event.type) {
    // The user's decision on a permission request, which reached the agent as this message.
    case 'permission': {
      const action = permissionAction(event)
      return plain(event.outcome === 'allowed'
        ? t('transcript.receipt.allowed', { action })
        : t('transcript.receipt.denied', { action }))
    }
    // The user's move of the conversation, which the user told the agent of.
    case 'moved':
      return plain(t('transcript.receipt.moved', { host: event.host, path: homePath(event.path, event.home) }))
    // Demi's notice that a move the agent asked for failed, and why, as the row's own words.
    case 'move_failed':
      return plain(t('transcript.receipt.moveFailed', {
        reason: (event.code ? codeText(event.code) : null) ?? event.reason,
      }))
    // A message from the main agent reaches one of its children.
    case 'message':
      return message.sender?.number === 0 ? mainAgentMessage() : named(t('transcript.receipt.fromAgent'))
    case 'completion':
      switch (event.outcome) {
        case 'completed':
          return named(t('transcript.receipt.finished'))
        case 'failed':
          return named(t('transcript.receipt.failed'), 'failed')
        case 'aborted':
          return named(t('transcript.receipt.stopped'), 'stopped')
      }
  }
}

/**
 * *Message from main agent*, its title inside the sentence: the catalogue's
 * one sentence is split at the agent, so a language that puts the agent
 * before its words (*メインエージェントからのメッセージ*) is worded as it reads.
 */
function mainAgentMessage(): ReceiptRow {
  const [before, after] = around('transcript.receipt.fromMain', 'agent')
  return { notice: before, title: t('transcript.receipt.mainAgent'), inline: true, after, tag: null }
}

/**
 * What a permission decision let the conversation do, in the page's words: its
 * categories' actions and its host's, or the use of a saved credential; the
 * English the message recorded for a category the page has no words for, or for
 * a decision recorded before ids were, which names neither.
 */
function permissionAction(event: Extract<AgentMessage['event'], { type: 'permission' }>): string {
  if (event.credential) {
    return useCredentialAction(event.credential)
  }
  const host = event.pairing ? [hostAction({ kind: 'pairing' })] : event.host ? [reachAction(event.host)] : []
  const categories = event.categories ?? []
  return (categories.length || host.length ? wordedActions(categories, host) : null) ?? event.action
}

/** The agent a message names: its description, or its number when it has none. */
function senderName(message: AgentMessage): string {
  const sender = message.sender
  return sender?.description.trim() || t('transcript.receipt.unnamed', { number: sender?.number ?? '' }).trim()
}

/**
 * Where a receipt came from: the sender's transcript, null for the
 * conversation's own (the root, agent 0), and the block of it the receipt
 * names, if any: an update's call that sent it; a completion's last
 * answer, or a failed child's failure record.
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

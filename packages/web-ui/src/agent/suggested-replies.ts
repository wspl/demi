import { inject, provide, type InjectionKey } from 'vue'

/**
 * The replies the user is likely to send next, which the conversation keeps
 * with the answer they follow: one row of pills under that answer's footer
 * (`SuggestedReplies`), while the conversation rests after it.
 */
export interface SuggestedReplies {
  /** The text block the suggestions follow. */
  blockId: string
  /** Null while they are being written; empty when none came. */
  replies: readonly SuggestedReply[] | null
}

/**
 * One suggestion: an answer to what the reply asked or offered (an approval,
 * a choice, an answer to its question), or a new topic the user may raise
 * next, as the small model judged it.
 */
export interface SuggestedReply {
  text: string
  kind: 'answer' | 'topic'
}

/** What the transcript shows of the suggestions and what a pill does. */
export interface SuggestedRepliesSource {
  /** The suggestions shown now; null where the conversation has none. */
  current(): SuggestedReplies | null
  /** Sends `text` as a new message, alone: the composer's draft stays. */
  send(text: string): void
  /** Puts `text` into the composer to edit before sending. */
  edit(text: string): void
}

const suggestedRepliesKey: InjectionKey<SuggestedRepliesSource> = Symbol('suggested-replies')

/** The conversation's surface gives its transcript the suggestions and their actions. */
export function provideSuggestedReplies(source: SuggestedRepliesSource): void {
  provide(suggestedRepliesKey, source)
}

/** The suggestions of the transcript an answer stands in; null outside a surface that has them. */
export function useSuggestedReplies(): SuggestedRepliesSource | null {
  return inject(suggestedRepliesKey, null)
}

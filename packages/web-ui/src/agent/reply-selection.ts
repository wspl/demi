import { inject, provide, type InjectionKey } from 'vue'
import type { ContextMenuEntry } from '../ui/context-menu'

/**
 * Tools the transcript adds for text selected in an agent's reply, after
 * Copy and before Send to Conversation, such as Comment (`ReplyComments`),
 * for the reply's text block `blockId`, whose element is `root`.
 * `streaming` reads whether the reply is still arriving now, where a tool
 * that needs its whole text is disabled; read where the tool is drawn, so it
 * turns enabled as the reply ends under a selection that stays. A page that
 * provides none adds nothing.
 */
export type ReplySelectionTools = (
  reply: { blockId: string; root: HTMLElement },
  streaming: () => boolean,
) => readonly ContextMenuEntry[]

const replySelectionKey: InjectionKey<ReplySelectionTools> = Symbol('replySelectionTools')

export function provideReplySelectionTools(tools: ReplySelectionTools): void {
  provide(replySelectionKey, tools)
}

export function useReplySelectionTools(): ReplySelectionTools | null {
  return inject(replySelectionKey, null)
}

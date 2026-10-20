import { inject, provide, type InjectionKey } from 'vue'
import type { ContextMenuEntry } from '../ui/context-menu'

/**
 * Tools the transcript adds for what the user picks in an agent's reply, the
 * reply's text block `blockId`, whose element is `root`, such as Comment and
 * Ask (`ReplyComments`):
 *
 * - for selected text, after Copy and before Send to Conversation
 * - for a picture, after Copy Image and Save Image and before Send to
 *   Conversation
 *
 * `streaming` reads whether the reply is still arriving now, where a tool that
 * needs its whole text is disabled. It is read where the tool is drawn, so it
 * turns enabled as the reply ends under a selection that stays. A page that
 * provides none adds nothing.
 */
export interface ReplySelectionTools {
  selection: (reply: { blockId: string; root: HTMLElement }, streaming: () => boolean) => readonly ContextMenuEntry[]
  picture: (reply: { blockId: string; root: HTMLElement }, image: HTMLImageElement, streaming: () => boolean) => readonly ContextMenuEntry[]
}

const replySelectionKey: InjectionKey<ReplySelectionTools> = Symbol('replySelectionTools')

export function provideReplySelectionTools(tools: ReplySelectionTools): void {
  provide(replySelectionKey, tools)
}

export function useReplySelectionTools(): ReplySelectionTools | null {
  return inject(replySelectionKey, null)
}

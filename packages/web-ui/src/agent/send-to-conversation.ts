import { h, inject, provide, type InjectionKey } from 'vue'
import { Send } from '@lucide/vue'
import type { SearchRow, SearchSource } from '../search/search'
import type { SidebarConversation } from '../sidebar/types'
import type { ContextMenuEntry } from '../ui/context-menu'
import type { CopiedMessage } from './message-input/copied-message'
import SendToConversationMenu from './SendToConversationMenu.vue'
import { t } from './locales/text'

/**
 * Send to Conversation (`web-application.md` § Context menus): words of
 * one conversation go to the end of another's draft, as pasting them there
 * would add them, and nothing is sent. The page says where they can go.
 */
export interface SendTargets {
  /** The conversations the user was last active in, most recent first; the search window's recent list. */
  recent: () => readonly SearchRow[]
  /** The search window's search, over every conversation, the archived ones included. */
  search: SearchSource
  /**
   * How the sidebar draws the conversation `id`: what its status dot shows,
   * and the name of its project, null outside one; null for one the page
   * does not list, as an archived one a search found.
   */
  describe: (id: string) => SendTargetLook | null
  /** The conversation shown, which the words come from; null outside one. */
  shown: () => string | null
  /**
   * Opens the conversation `to`, or a new one in the shown one's project
   * for null, with `words` added to the end of its draft, as a copied
   * message pastes.
   */
  send: (to: string | null, words: CopiedMessage) => void
}

/** A conversation as its sidebar row shows it: its dot's state and its project's name. */
export interface SendTargetLook {
  status: Pick<SidebarConversation, 'status' | 'needsYou' | 'unread'>
  project: string | null
}

/** How many conversations the submenu lists before the user types. */
export const SEND_RECENT = 8

const sendTargetsKey: InjectionKey<SendTargets> = Symbol('sendTargets')

export function provideSendTargets(targets: SendTargets): void {
  provide(sendTargetsKey, targets)
}

/** Where words can go; none where the page offers no conversations, as on a share's page. */
export function useSendTargets(): SendTargets | null {
  return inject(sendTargetsKey, null)
}

/** The conversations the submenu lists while its field is empty: the last active, leaving out the shown one and the archived ones. */
export function restingTargets(targets: SendTargets): SearchRow[] {
  const shown = targets.shown()
  return targets.recent().filter((row) => !row.archived && row.conversationId !== shown).slice(0, SEND_RECENT)
}

/** Text as a paste of it brings it: the words alone, files none. */
export function textWords(text: string): CopiedMessage {
  return { text, files: [] }
}

/** Text of the agent's reply as a Markdown quotation, each line quoted. */
export function quotedWords(text: string): CopiedMessage {
  return textWords(text.replace(/\n+$/, '').split('\n').map((line) => (line === '' ? '>' : `> ${line}`)).join('\n'))
}

/** The Send to Conversation row, its submenu the search and the conversations; `words` is read as one is picked. */
export function sendEntry(targets: SendTargets, words: () => CopiedMessage | Promise<CopiedMessage>): ContextMenuEntry {
  return {
    icon: Send,
    label: t('transcript.sendToConversation'),
    submenu: () => h(SendToConversationMenu, { targets, words }),
  }
}

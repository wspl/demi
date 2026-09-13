import { reactive } from 'vue'
import { defineStore } from 'pinia'
import {
  nextActiveWorkTab,
  type WorkTab,
} from '@demicodes/web-ui/agent/work-panel'

interface ConversationWorkTabs {
  tabs: WorkTab[]
  activeId: string | null
}

/**
 * The work panel's tabs, kept per conversation for the life of the page.
 * Nothing opens a tab yet: file and diff views are the next step, and the
 * panel shows its empty state until then.
 */
export const useWorkPanel = defineStore('workPanel', () => {
  const byConversation = reactive(new Map<string, ConversationWorkTabs>())

  function entry(conversationId: string | null): ConversationWorkTabs | null {
    if (conversationId === null) {
      return null
    }
    return byConversation.get(conversationId) ?? null
  }

  function tabsFor(conversationId: string | null): WorkTab[] {
    return entry(conversationId)?.tabs ?? []
  }

  function activeIdFor(conversationId: string | null): string | null {
    return entry(conversationId)?.activeId ?? null
  }

  function select(conversationId: string | null, tabId: string): void {
    const current = entry(conversationId)
    if (current) {
      current.activeId = tabId
    }
  }

  function closeTab(conversationId: string | null, tabId: string): void {
    const current = entry(conversationId)
    if (!current) {
      return
    }
    if (current.activeId === tabId) {
      current.activeId = nextActiveWorkTab(current.tabs, tabId)
    }
    current.tabs = current.tabs.filter((tab) => tab.id !== tabId)
  }

  return { tabsFor, activeIdFor, select, closeTab }
})

import { reactive } from 'vue'
import { defineStore } from 'pinia'
import type { CallEditSelection, ChangeMode, ReadCallChange } from '@demicodes/web-ui/files/changes'
import {
  workPanelTabs,
  addBrowserTab,
  closeBrowserTabs,
  goBackInTab,
  goForwardInTab,
  showChangeInTab,
  showCallEdit,
  showFileInTab,
  type WorkTab,
  type BrowserPage,
  type BrowserWorkTab,
  type ChangeWorkTab,
} from '@demicodes/web-ui/agent/work-panel'
import { useResources } from '../state/resources'
import { createCallChangeReader, createWorkingTreeSource, type WorkingTreeSource } from './changes'

/** One conversation's work panel: whether it is open, its tabs, the active one, and its working tree. */
export interface WorkState {
  open: boolean
  tabs: WorkTab[]
  activeId: string | null
  readCallChange: ReadCallChange
  changes: WorkingTreeSource
}

/**
 * The work panel's state per conversation, for the page's lifetime: whether
 * the reader has it open beside that conversation, its tabs and their
 * selections, and the working-tree source behind its Change tab. The open flag
 * reads and writes account-local preferences; tab selections remain in memory.
 */
export const useWorkPanel = defineStore('work-panel', () => {
  const resources = useResources()
  const states = reactive(new Map<string, WorkState>())

  function stateFor(conversationId: string): WorkState {
    if (!states.has(conversationId)) {
      states.set(conversationId, {
        get open(): boolean {
          return resources.local.workPanelOpen?.[conversationId] ?? false
        },
        set open(open: boolean) {
          resources.local.workPanelOpen ??= {}
          resources.local.workPanelOpen[conversationId] = open
        },
        tabs: workPanelTabs(),
        activeId: 'change',
        readCallChange: createCallChangeReader(conversationId),
        changes: createWorkingTreeSource(conversationId),
      })
    }
    // The map's own (reactive) view of the entry, never the plain object it was made from.
    return states.get(conversationId)!
  }

  function setOpen(state: WorkState, open: boolean): void {
    state.open = open
  }

  function select(state: WorkState, id: string): void {
    state.activeId = id
  }

  /** A new browser tab, selected: empty, or showing `page` with the panel opened for it. */
  function addBrowser(state: WorkState, page?: BrowserPage): void {
    const next = addBrowserTab(state.tabs, page)
    state.tabs = next.tabs
    state.activeId = next.activeId
    if (page) {
      state.open = true
    }
  }

  function closeTabs(state: WorkState, ids: string[]): void {
    const next = closeBrowserTabs(state.tabs, state.activeId, ids)
    state.tabs = next.tabs
    state.activeId = next.activeId
  }

  function updateBrowser(state: WorkState, tab: BrowserWorkTab): void {
    state.tabs = state.tabs.map((current) => current.id === tab.id ? tab : current)
  }

  /** A file by Host path, shown in the File section, with the panel opened for it. */
  function open(state: WorkState, path: string): void {
    const next = showFileInTab(state.tabs, path)
    state.tabs = next.tabs
    state.activeId = next.activeId
    state.open = true
  }

  function showChange(state: WorkState, id: string, mode: ChangeMode, path: string | null, selection?: { call: ChangeWorkTab['call']; edit: number }): void {
    state.tabs = showChangeInTab(state.tabs, id, mode, path, selection)
  }

  function selectEdit(state: WorkState, selection: CallEditSelection): void {
    const next = showCallEdit(state.tabs, selection)
    state.tabs = next.tabs
    state.activeId = next.activeId
    state.open = true
  }

  function back(state: WorkState, id: string): void {
    state.tabs = goBackInTab(state.tabs, id)
  }

  function forward(state: WorkState, id: string): void {
    state.tabs = goForwardInTab(state.tabs, id)
  }

  return { stateFor, setOpen, select, addBrowser, closeTabs, updateBrowser, open, showChange, selectEdit, back, forward }
})

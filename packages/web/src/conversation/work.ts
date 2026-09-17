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
  type BrowserWorkTab,
  type ChangeWorkTab,
} from '@demicodes/web-ui/agent/work-panel'
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
 * selections, and the working-tree source behind its Change tab. Nothing here
 * persists; a reload starts every conversation's panel closed on Uncommitted.
 */
export const useWorkPanel = defineStore('work-panel', () => {
  const states = reactive(new Map<string, WorkState>())

  function stateFor(conversationId: string): WorkState {
    if (!states.has(conversationId)) {
      states.set(conversationId, {
        open: false,
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

  function addBrowser(state: WorkState): void {
    const next = addBrowserTab(state.tabs)
    state.tabs = next.tabs
    state.activeId = next.activeId
  }

  function closeTabs(state: WorkState, ids: string[]): void {
    const next = closeBrowserTabs(state.tabs, state.activeId, ids)
    state.tabs = next.tabs
    state.activeId = next.activeId
  }

  function updateBrowser(state: WorkState, tab: BrowserWorkTab): void {
    state.tabs = state.tabs.map((current) => current.id === tab.id ? tab : current)
  }

  /** A file by workspace path, shown in the active tab in place. */
  function open(state: WorkState, path: string): void {
    const next = showFileInTab(state.tabs, path)
    state.tabs = next.tabs
    state.activeId = next.activeId
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

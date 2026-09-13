import { reactive } from 'vue'
import { defineStore } from 'pinia'
import type { ChangeMode } from '@demicodes/web-ui/files/changes'
import {
  changeWorkTab,
  closeWorkTabs,
  fileWorkTab,
  findChangeWorkTab,
  goBackInTab,
  goForwardInTab,
  showChangeInTab,
  showFileInTab,
  type WorkTab,
} from '@demicodes/web-ui/agent/work-panel'
import { createWorkingTreeSource, type WorkingTreeSource } from './changes'

/** One conversation's work panel: whether it is open, its tabs, the active one, and its working tree. */
export interface WorkState {
  open: boolean
  tabs: WorkTab[]
  activeId: string | null
  changes: WorkingTreeSource
}

/**
 * The work panel's state per conversation, for the page's lifetime: whether
 * the reader has it open beside that conversation, the tabs they opened
 * there, and the working-tree source behind its Change tab. Nothing here
 * persists; a reload starts every conversation's panel closed and empty.
 */
export const useWorkPanel = defineStore('work-panel', () => {
  const states = reactive(new Map<string, WorkState>())
  let nextId = 1

  function stateFor(conversationId: string): WorkState {
    if (!states.has(conversationId)) {
      states.set(conversationId, {
        open: false,
        tabs: [],
        activeId: null,
        changes: createWorkingTreeSource(conversationId),
      })
    }
    // The map's own (reactive) view of the entry, never the plain object it was made from.
    return states.get(conversationId)!
  }

  function newId(): string {
    nextId += 1
    return `work-${nextId}`
  }

  function setOpen(state: WorkState, open: boolean): void {
    state.open = open
  }

  function select(state: WorkState, id: string): void {
    state.activeId = id
  }

  function close(state: WorkState, ids: string[]): void {
    const next = closeWorkTabs(state.tabs, state.activeId, ids)
    state.tabs = next.tabs
    state.activeId = next.activeId
  }

  /** A new file tab on `path` (relative to the workspace root). */
  function addFile(state: WorkState, path: string): void {
    const tab = fileWorkTab(newId(), path)
    state.tabs = [...state.tabs, tab]
    state.activeId = tab.id
  }

  /** The one change tab, in `mode`: selected when it exists, opened otherwise. */
  function addChange(state: WorkState, mode: ChangeMode): void {
    const open = findChangeWorkTab(state.tabs)
    if (open) {
      state.activeId = open.id
      return
    }
    const tab = changeWorkTab(newId(), mode)
    state.tabs = [...state.tabs, tab]
    state.activeId = tab.id
  }

  /** A file by workspace path, shown in the active tab in place. */
  function open(state: WorkState, path: string): void {
    const next = showFileInTab(state.tabs, state.activeId, path, newId)
    state.tabs = next.tabs
    state.activeId = next.activeId
  }

  function showChange(state: WorkState, id: string, mode: ChangeMode, path: string | null): void {
    state.tabs = showChangeInTab(state.tabs, id, mode, path)
  }

  function back(state: WorkState, id: string): void {
    state.tabs = goBackInTab(state.tabs, id)
  }

  function forward(state: WorkState, id: string): void {
    state.tabs = goForwardInTab(state.tabs, id)
  }

  return { stateFor, setOpen, select, close, addFile, addChange, open, showChange, back, forward }
})

import { reactive } from 'vue'
import { defineStore } from 'pinia'
import type { CallEditSelection, ChangeMode, ReadCallChange } from '@demicodes/web-ui/files/changes'
import {
  workPanelTabs,
  goBackInTab,
  goForwardInTab,
  showChangeInTab,
  showCallEdit,
  showFileInTab,
  type WorkTab,
  type ChangeWorkTab,
} from '@demicodes/web-ui/agent/work-panel'
import {
  addTab,
  emptyPanelState,
  removeTabs,
  selectInPanel,
  updateTab,
  type PanelState,
} from '@demicodes/web-ui/agent/panel-tabs'
import { loadPanel, savePanel } from '../api/panel'
import { reportError } from '@demicodes/web-ui/infra/errors'
import { useResources } from '../state/resources'
import { createCallChangeReader, createWorkingTreeSource, type WorkingTreeSource } from './changes'

/** One conversation's work panel: whether it is open, its fixed views, its saved selection and tabs, and its working tree. */
export interface WorkState {
  open: boolean
  /** The fixed Change and File views' own state. */
  views: WorkTab[]
  /** The selection and the user's tabs, saved with the conversation. */
  panel: PanelState
  /** The saved panel has been read; nothing is saved over it before. */
  loaded: boolean
  /** The user changed the panel before it was read; the read keeps those changes. */
  changedBeforeLoad: boolean
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
        views: workPanelTabs(),
        panel: emptyPanelState(),
        loaded: false,
        changedBeforeLoad: false,
        readCallChange: createCallChangeReader(conversationId),
        changes: createWorkingTreeSource(conversationId),
      })
    }
    // The map's own (reactive) view of the entry, never the plain object it was made from.
    return states.get(conversationId)!
  }

  /** Reads the saved panel once per page, and again when the page returns to it. */
  async function load(conversationId: string): Promise<void> {
    const state = stateFor(conversationId)
    try {
      const saved = await loadPanel(conversationId)
      const early = state.changedBeforeLoad ? state.panel : null
      state.loaded = true
      state.changedBeforeLoad = false
      if (early === null) {
        state.panel = saved
        return
      }
      // What the user did while the read was on its way stands: their tabs join the saved ones.
      const known = new Set(saved.tabs.map((tab) => tab.id))
      const added = early.tabs.filter((tab) => !known.has(tab.id))
      change(conversationId, { selection: early.selection, tabs: [...saved.tabs, ...added] })
    } catch (error) {
      reportError('Could not read the work panel', error, { userVisible: true })
    }
  }

  /** Every change applies to the page first and is then saved whole; the last save wins. */
  function change(conversationId: string, next: PanelState): void {
    const state = stateFor(conversationId)
    state.panel = next
    if (!state.loaded) {
      state.changedBeforeLoad = true
      return
    }
    savePanel(conversationId, next).catch((error: unknown) => {
      reportError('Could not save the work panel', error, { userVisible: true })
    })
  }

  function setOpen(state: WorkState, open: boolean): void {
    state.open = open
  }

  function select(conversationId: string, selection: string): void {
    change(conversationId, selectInPanel(stateFor(conversationId).panel, selection))
  }

  /** A new tab, selected, with the panel opened for it; returns its id. */
  function add(conversationId: string, kind: string, data: unknown, options = { select: true }): string {
    const state = stateFor(conversationId)
    const added = addTab(state.panel, { kind, data }, options)
    change(conversationId, added.state)
    if (options.select) {
      state.open = true
    }
    return added.id
  }

  function update(conversationId: string, id: string, data: unknown): void {
    change(conversationId, updateTab(stateFor(conversationId).panel, id, data))
  }

  function closeTabs(conversationId: string, ids: string[]): void {
    change(conversationId, removeTabs(stateFor(conversationId).panel, ids))
  }

  /** A file by Host path, shown in the File view, with the panel opened for it. */
  function open(conversationId: string, path: string): void {
    const state = stateFor(conversationId)
    const next = showFileInTab(state.views, path)
    state.views = next.tabs
    if (next.activeId !== null) {
      select(conversationId, next.activeId)
    }
    state.open = true
  }

  function showChange(state: WorkState, id: string, mode: ChangeMode, path: string | null, selection?: { call: ChangeWorkTab['call']; edit: number }): void {
    state.views = showChangeInTab(state.views, id, mode, path, selection)
  }

  function selectEdit(conversationId: string, selection: CallEditSelection): void {
    const state = stateFor(conversationId)
    const next = showCallEdit(state.views, selection)
    state.views = next.tabs
    select(conversationId, next.activeId)
    state.open = true
  }

  function back(state: WorkState, id: string): void {
    state.views = goBackInTab(state.views, id)
  }

  function forward(state: WorkState, id: string): void {
    state.views = goForwardInTab(state.views, id)
  }

  return { stateFor, setOpen, load, select, add, update, closeTabs, open, showChange, selectEdit, back, forward }
})

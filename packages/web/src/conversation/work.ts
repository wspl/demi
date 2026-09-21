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
  /** The one read of the saved panel has started; it is not asked for again once it answered. */
  loading: boolean
  /** The saved panel has been read; nothing is saved over it before. */
  loaded: boolean
  /** Counts the page's own changes, so the read knows whether the user acted before it arrived. */
  revision: number
  /** A save is on its way; `unsaved` says the panel changed again since it left. */
  saving: boolean
  unsaved: boolean
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
        loading: false,
        loaded: false,
        revision: 0,
        saving: false,
        unsaved: false,
        readCallChange: createCallChangeReader(conversationId),
        changes: createWorkingTreeSource(conversationId),
      })
    }
    // The map's own (reactive) view of the entry, never the plain object it was made from.
    return states.get(conversationId)!
  }

  /**
   * Reads the saved panel, once per conversation in a page's life. From then
   * on the page's own state is the newest there is: everything it changes it
   * saves, so reading again could only bring back something older. What the
   * user did before the read arrived stays, beside the saved tabs.
   */
  async function load(conversationId: string): Promise<void> {
    const state = stateFor(conversationId)
    if (state.loading) {
      return
    }
    state.loading = true
    const revision = state.revision
    let saved: PanelState
    try {
      saved = await loadPanel(conversationId)
    } catch (error) {
      // Not read: the next opening of the panel tries again.
      state.loading = false
      reportError('Could not read the work panel', error, { userVisible: true })
      return
    }
    state.loaded = true
    if (state.revision === revision) {
      state.panel = saved
      return
    }
    const known = new Set(saved.tabs.map((tab) => tab.id))
    const added = state.panel.tabs.filter((tab) => !known.has(tab.id))
    change(conversationId, { selection: state.panel.selection, tabs: [...saved.tabs, ...added] })
  }

  /** Every change applies to the page first and is then saved whole. */
  function change(conversationId: string, next: PanelState): void {
    const state = stateFor(conversationId)
    state.panel = next
    state.revision += 1
    if (state.loaded) {
      void save(conversationId)
    }
  }

  /**
   * One save at a time, always of the latest panel: saves that overlap could
   * arrive out of order and leave an older panel as the saved one.
   */
  async function save(conversationId: string): Promise<void> {
    const state = stateFor(conversationId)
    state.unsaved = true
    if (state.saving) {
      return
    }
    state.saving = true
    try {
      while (state.unsaved) {
        state.unsaved = false
        await savePanel(conversationId, state.panel)
      }
    } catch (error) {
      // The panel stays as the page has it; the next change saves it whole again.
      reportError('Could not save the work panel', error, { userVisible: true })
    } finally {
      state.saving = false
    }
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

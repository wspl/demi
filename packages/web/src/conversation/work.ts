import { reactive } from 'vue'
import { defineStore } from 'pinia'
import {
  addTab,
  emptyPanelState,
  openIntent,
  removeTabs,
  selectInPanel,
  updateTab,
  type PanelState,
  type PinnedTabs,
} from '@demicodes/web-ui/agent/panel-tabs'
import { intentKind } from '@demicodes/web-ui/plugins/page'
import type { IntentName, IntentRequest } from '@demicodes/web-ui/plugins/intents'
import { loadPanel, savePanel } from '../api/panel'
import { reportError } from '@demicodes/web-ui/infra/errors'
import { useResources } from '../state/resources'
import { useProduct } from '../state/product'
import { PLUGIN_PAGES } from '../plugins/generated/pages'
import { pluginEnabled } from '../plugins/enabled'
import { createWorkingTreeSource, type WorkingTreeSource } from './changes'

/** One conversation's work panel: whether it is open, its pinned tabs, its saved selection and tabs, and its working tree. */
export interface WorkState {
  open: boolean
  /** The pinned tabs' data, by kind, for the page's lifetime. */
  pinned: PinnedTabs
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
  changes: WorkingTreeSource
}

/**
 * The work panel's state per conversation, for the page's lifetime: whether
 * the reader has it open beside that conversation, its tabs and their
 * pinned tabs' data, and the working-tree source behind its files service.
 * The open flag reads and writes account-local preferences; pinned tabs'
 * data remains in memory.
 */
export const useWorkPanel = defineStore('work-panel', () => {
  const resources = useResources()
  const product = useProduct()
  const enabled = (plugin: string) => pluginEnabled(product.snapshot, plugin)
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
        pinned: {},
        panel: emptyPanelState(),
        loading: false,
        loaded: false,
        revision: 0,
        saving: false,
        unsaved: false,
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

  function select(conversationId: string, selection: string | null): void {
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

  /** A pinned tab's data, as its kind replaces it. */
  function updatePinned(conversationId: string, kind: string, data: unknown): void {
    const state = stateFor(conversationId)
    state.pinned = { ...state.pinned, [kind]: data }
  }

  /**
   * Opens an intent in the conversation's panel (`plugin-pages.md`
   * § Intents): the kind that opens it shows it, in its pinned tab or a new
   * one, which takes the selection, with the panel opened for it.
   */
  function openIn(conversationId: string, request: IntentRequest): void {
    const state = stateFor(conversationId)
    const opened = openIntent({ state: state.panel, pinned: state.pinned }, PLUGIN_PAGES, enabled, request)
    if (!opened) {
      return
    }
    state.pinned = opened.pinned
    change(conversationId, opened.state)
    state.open = true
  }

  /** Whether any page the user has on opens `intent`. */
  function canOpen(intent: IntentName): boolean {
    return intentKind(PLUGIN_PAGES, enabled, intent) !== null
  }

  return { stateFor, setOpen, load, select, add, update, closeTabs, updatePinned, openIn, canOpen }
})

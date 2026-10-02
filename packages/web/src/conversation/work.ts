import { reactive, watch } from 'vue'
import { defineStore } from 'pinia'
import {
  openIntent,
  removeTabs,
  type PanelState,
  type PinnedTabs,
} from '@demicodes/web-ui/agent/panel-tabs'
import { PanelTabs, dataChanges } from '@demicodes/web-ui/agent/panel-changes'
import { intentKind } from '@demicodes/web-ui/plugins/page'
import type { IntentName, IntentRequest } from '@demicodes/web-ui/plugins/intents'
import { panelBackend } from '../api/panel'
import { reportError } from '@demicodes/web-ui/infra/errors'
import { useResources } from '../state/resources'
import { useProduct } from '../state/product'
import { PLUGIN_PAGES } from '../plugins/generated/pages'
import { pluginEnabled } from '../plugins/enabled'
import { createWorkingTreeSource, type WorkingTreeSource } from './changes'
import { useConversations } from './store'

/**
 * One conversation's work panel: whether it is open and what it selects,
 * both this page's own, its pinned tabs' data, the backend's tabs with this
 * page's changes over them, and its working tree.
 */
export interface WorkState {
  open: boolean
  /** A tab's id or a pinned kind's id; null names nothing. */
  selection: string | null
  /** The pinned tabs' data, by kind, for the page's lifetime. */
  pinned: PinnedTabs
  /** The selection and the tabs, as the panel shows them. */
  readonly panel: PanelState
  changes: WorkingTreeSource
}

/** Whether `data` is an object a kind's tab keeps as its `data`. */
function isData(data: unknown): data is Record<string, unknown> {
  return data !== null && typeof data === 'object' && !Array.isArray(data)
}

/**
 * The work panel's state per conversation, for the page's lifetime
 * (`web-application.md` § Work panel): whether the reader has it open beside
 * that conversation and what it selects, kept in the account's local
 * preferences; its pinned tabs' data, in memory; the backend's tabs, read
 * when their revision rises, with this page's changes shown at once over
 * them; and the working-tree source behind its files service.
 */
export const useWorkPanel = defineStore('work-panel', () => {
  const resources = useResources()
  const product = useProduct()
  const conversations = useConversations()
  const enabled = (plugin: string) => pluginEnabled(product.snapshot, plugin)
  const states = reactive(new Map<string, WorkState>())
  /** Each conversation's tabs; reactive on their own, so they stand beside the states, not in them. */
  const panels = new Map<string, PanelTabs>()

  /** The conversation's tabs, made the first time they are asked for. */
  function tabsOf(conversationId: string): PanelTabs {
    let tabs = panels.get(conversationId)
    if (!tabs) {
      tabs = new PanelTabs(panelBackend(conversationId), (error) => {
        reportError('Could not change the work panel', error, { userVisible: true })
      })
      panels.set(conversationId, tabs)
    }
    return tabs
  }

  function stateFor(conversationId: string): WorkState {
    if (!states.has(conversationId)) {
      const tabs = tabsOf(conversationId)
      states.set(conversationId, {
        get open(): boolean {
          return resources.local.workPanelOpen?.[conversationId] ?? false
        },
        set open(open: boolean) {
          resources.local.workPanelOpen ??= {}
          resources.local.workPanelOpen[conversationId] = open
        },
        get selection(): string | null {
          return resources.local.workPanelSelection?.[conversationId] ?? null
        },
        set selection(selection: string | null) {
          resources.local.workPanelSelection ??= {}
          if (selection === null) {
            delete resources.local.workPanelSelection[conversationId]
          } else {
            resources.local.workPanelSelection[conversationId] = selection
          }
        },
        pinned: {},
        get panel(): PanelState {
          return { selection: this.selection, tabs: tabs.tabs.value }
        },
        changes: createWorkingTreeSource(conversationId),
      })
    }
    // The map's own (reactive) view of the entry, never the plain object it was made from.
    return states.get(conversationId)!
  }

  // A panel a page has read is read again when the summary says it changed.
  watch(
    () => product.snapshot?.conversations,
    (summaries) => {
      for (const summary of summaries ?? []) {
        const tabs = panels.get(summary.id)
        if (tabs?.read && summary.panelRevision > tabs.revision) {
          void tabs.refresh()
        }
      }
    },
  )

  /**
   * Whether the conversation has its backend record, which its first send
   * creates. Before it, the panel binds no page and reads or sends nothing
   * (`web-application.md` § Work panel).
   */
  function recorded(conversationId: string): boolean {
    return conversations.items.find((item) => item.id === conversationId)?.persistence === 'synced'
  }

  /** Reads the panel and sends the page's changes, once the conversation has its record. */
  function load(conversationId: string): void {
    if (recorded(conversationId)) {
      tabsOf(conversationId).start()
    }
  }

  function setOpen(state: WorkState, open: boolean): void {
    state.open = open
  }

  function select(conversationId: string, selection: string | null): void {
    stateFor(conversationId).selection = selection
  }

  /** A new tab, selected unless `options` says not, with the panel opened for it; returns its id. */
  function add(conversationId: string, kind: string, data: unknown, options = { select: true }): string {
    const state = stateFor(conversationId)
    const id = crypto.randomUUID()
    tabsOf(conversationId).change({ type: 'create', tab: { id, kind, data } })
    if (options.select) {
      state.selection = id
      state.open = true
    }
    return id
  }

  /** The tab's next `data`, from its kind: only what changed is sent. */
  function update(conversationId: string, id: string, data: unknown): void {
    const tabs = tabsOf(conversationId)
    const current = tabs.tabs.value.find((tab) => tab.id === id)
    if (!current || !isData(data)) {
      return
    }
    const fields = dataChanges(current.data, data)
    if (Object.keys(fields).length > 0) {
      tabs.change({ type: 'update', id, data: fields })
    }
  }

  /** Closes the tabs `ids`; a closed selection passes to its nearest neighbour. */
  function closeTabs(conversationId: string, ids: string[]): void {
    const state = stateFor(conversationId)
    state.selection = removeTabs(state.panel, ids).selection
    for (const id of ids) {
      tabsOf(conversationId).change({ type: 'remove', id })
    }
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
    const opened = openIntent(state.pinned, PLUGIN_PAGES, enabled, request)
    if (!opened) {
      return
    }
    state.pinned = opened.pinned
    if (opened.created) {
      tabsOf(conversationId).change({ type: 'create', tab: opened.created })
    }
    state.selection = opened.selection
    state.open = true
  }

  /** Whether any page the user has on opens `intent`. */
  function canOpen(intent: IntentName): boolean {
    return intentKind(PLUGIN_PAGES, enabled, intent) !== null
  }

  return { stateFor, recorded, setOpen, load, select, add, update, closeTabs, updatePinned, openIn, canOpen }
})

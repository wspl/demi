import { reactive, watch } from 'vue'
import { defineStore } from 'pinia'
import {
  openIntent,
  pendingShows,
  type PanelState,
  type PanelTab,
  type PinnedTabs,
} from '@demicodes/web-ui/agent/panel-tabs'
import { PanelTabs, closePanelTabs, updatePanelTab } from '@demicodes/web-ui/agent/panel-changes'
import { selectTab } from '@demicodes/web-ui/agent/tab-close'
import { intentKind } from '@demicodes/web-ui/plugins/page'
import type { IntentName, IntentRequest } from '@demicodes/web-ui/plugins/intents'
import { panelBackend } from '../api/panel'
import { reportError } from '@demicodes/web-ui/infra/errors'
import { useResources } from '../state/resources'
import { useProduct } from '../state/product'
import { PLUGIN_PAGES } from '../plugins/generated/pages'
import { pluginEnabled } from '../plugins/enabled'
import { useConversations } from './store'

/**
 * One conversation's work panel: whether it is open and what it selects,
 * both this page's own, its pinned tabs' data, and the backend's tabs with
 * this page's changes over them.
 */
export interface WorkState {
  open: boolean
  /** The tabs' and pinned kinds' ids this page selected, the newest last. */
  history: readonly string[]
  /** The pinned tabs' data, by kind, for the page's lifetime. */
  pinned: PinnedTabs
  /** The history and the tabs, as the panel shows them. */
  readonly panel: PanelState
}

/**
 * The work panel's state per conversation, for the page's lifetime
 * (`web-application.md` § Work panel): whether the reader has it open beside
 * that conversation and what it selects, kept in the account's local
 * preferences; its pinned tabs' data, in memory; the backend's tabs, read
 * when their revision rises, with this page's changes shown at once over
 * them.
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
      const created = new PanelTabs(panelBackend(conversationId), (error) => {
        reportError('Could Not Change the Work Panel', error, { userVisible: true })
      })
      panels.set(conversationId, created)
      // A tab whose kind asks that the user see it opens the panel, open or closed (`web-application.md` § Work panel).
      watch(() => created.tabs.value, (current) => applyShows(conversationId, current))
      tabs = created
    }
    return tabs
  }

  /**
   * Opens the panel on each tab whose kind counts more showings than this
   * page applied, once, and records the counts beside the selection history.
   */
  function applyShows(conversationId: string, tabs: readonly PanelTab[]): void {
    const applied = resources.local.workPanelShown?.[conversationId] ?? {}
    const pending = pendingShows(tabs, PLUGIN_PAGES, enabled, applied)
    if (!pending) {
      return
    }
    resources.local.workPanelShown ??= {}
    resources.local.workPanelShown[conversationId] = { ...pending.applied }
    const state = stateFor(conversationId)
    for (const id of pending.shown) {
      state.history = selectTab(state.history, id)
    }
    state.open = true
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
        get history(): readonly string[] {
          return resources.local.workPanelHistory?.[conversationId] ?? []
        },
        set history(history: readonly string[]) {
          resources.local.workPanelHistory ??= {}
          if (history.length === 0) {
            delete resources.local.workPanelHistory[conversationId]
          } else {
            resources.local.workPanelHistory[conversationId] = [...history]
          }
        },
        pinned: {},
        get panel(): PanelState {
          return { history: this.history, tabs: tabs.tabs.value }
        },
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
        panels.get(summary.id)?.noticed(summary.panelRevision)
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

  function select(conversationId: string, id: string): void {
    const state = stateFor(conversationId)
    state.history = selectTab(state.history, id)
  }

  /**
   * A new tab, after the others or at `options.index` among them, selected
   * unless `options` says not, with the panel opened for it; returns its id.
   */
  function add(
    conversationId: string,
    kind: string,
    data: unknown,
    options: { select: boolean; index?: number } = { select: true },
  ): string {
    const state = stateFor(conversationId)
    const id = crypto.randomUUID()
    tabsOf(conversationId).change({ type: 'create', tab: { id, kind, data }, index: options.index })
    if (options.select) {
      state.history = selectTab(state.history, id)
      state.open = true
    }
    return id
  }

  /** Moves the tab `id` to `index` among the others, where the user dragged it. */
  function move(conversationId: string, id: string, index: number): void {
    tabsOf(conversationId).change({ type: 'move', id, index })
  }

  /** The tab's next `data`, from its kind: only what changed is sent. */
  function update(conversationId: string, id: string, data: unknown): void {
    updatePanelTab(tabsOf(conversationId), id, data)
  }

  /** Closes the tabs `ids`; the panel shows what was selected before a closed one. */
  function closeTabs(conversationId: string, ids: string[]): void {
    closePanelTabs(stateFor(conversationId), tabsOf(conversationId), ids)
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
    state.history = selectTab(state.history, opened.selection)
    state.open = true
  }

  /** Whether any page the user has on opens `intent`. */
  function canOpen(intent: IntentName): boolean {
    return intentKind(PLUGIN_PAGES, enabled, intent) !== null
  }

  return { stateFor, recorded, setOpen, load, select, add, move, update, closeTabs, updatePinned, openIn, canOpen }
})

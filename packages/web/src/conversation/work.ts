import { reactive, shallowRef, watch } from 'vue'
import { defineStore } from 'pinia'
import { z } from 'zod'
import type { LocationQuery, RouteLocationNormalized, Router } from 'vue-router'
import {
  openIntent,
  pendingShows,
  pinnedData,
  type PanelState,
  type PanelTab,
  type PinnedTabs,
  type ShownPanel,
} from '@demicodes/web-ui/agent/panel-tabs'
import {
  addressSelection,
  panelAddress,
  placeData,
  sameAddress,
  type PanelAddress,
} from '@demicodes/web-ui/agent/panel-address'
import { PanelTabs, closePanelTabs, updatePanelTab } from '@demicodes/web-ui/agent/panel-changes'
import { selectTab } from '@demicodes/web-ui/agent/tab-close'
import { usePhone } from '@demicodes/web-ui/ui/phone'
import { intentKind, type HistoryDirection, type TabUpdate } from '@demicodes/web-ui/plugins/page'
import type { IntentName, IntentRequest } from '@demicodes/web-ui/plugins/intents'
import { panelBackend } from '../api/panel'
import { reportError } from '@demicodes/web-ui/infra/errors'
import { t } from './locales/text'
import { useResources } from '../state/resources'
import { useProduct } from '../state/product'
import { PLUGIN_PAGES } from '../plugins/generated/pages'
import { usePluginStates } from '../plugins/states'
import { pluginEnabled } from '../plugins/enabled'
import { useConversations } from './store'

/**
 * One conversation's work panel: whether it is open and what it selects,
 * both this page's own, its pinned tabs' data, and the backend's tabs with
 * this page's changes over them.
 */
export interface WorkState {
  /**
   * Whether the panel is open beside the conversation: while the page's
   * address shows the conversation, what the address says; otherwise, on
   * the desktop, the account's saved choice, and on a phone, where the panel
   * covers the conversation, closed.
   */
  readonly open: boolean
  /** The tabs' and pinned kinds' ids this page selected, the newest last. */
  readonly history: readonly string[]
  /** The pinned tabs' data, by kind, for the page's lifetime. */
  readonly pinned: PinnedTabs
  /** The history and the tabs, as the panel shows them. */
  readonly panel: PanelState
  /**
   * Whether the page read the conversation's tabs: the tabs the first read
   * brings were there before the page looked, and show at once.
   */
  readonly tabsRead: boolean
}

/**
 * The mark of a history entry whose address the panel wrote: an address
 * without `panel` then says the panel is closed, where an address nobody
 * marked, such as one the sidebar opens, shows the panel as the user left it.
 */
const ENTRY_MARK = 'workPanel'
/**
 * How many steps of the open panel an entry is from the one before the panel
 * opened, which a phone's Back to Conversation goes back to: 1 for the entry
 * that opened it. Null for a closed panel, or one an address nobody marked
 * opened, such as a link.
 */
const ENTRY_STEPS = 'workPanelSteps'

/**
 * What the panel reads of the router's current history entry: its mark and
 * steps, and the addresses of the entries before and after it, which
 * vue-router's web history keeps.
 */
const entryStateSchema = z.looseObject({
  back: z.string().nullable().optional(),
  forward: z.string().nullable().optional(),
  [ENTRY_MARK]: z.literal(true).optional(),
  [ENTRY_STEPS]: z.number().int().positive().nullable().optional(),
})

/** The conversation an address shows, and the panel's part of it. */
interface ShownAddress {
  conversation: string
  /** The address's path, which a move of the panel keeps. */
  path: string
  address: PanelAddress
  /** The panel wrote the address, so one without `panel` says the panel is closed. */
  marked: boolean
}

/** What a move of a conversation's panel leads to: open or closed, what it selected, and its pinned tabs' data. */
interface PanelView {
  open: boolean
  history: readonly string[]
  pinned: PinnedTabs
}

/** The conversation a route shows, or null for any other page. */
function routeConversation(route: Pick<RouteLocationNormalized, 'path' | 'params'>): string | null {
  const { id } = route.params
  return route.path.startsWith('/chat/') && typeof id === 'string' && id !== '' ? id : null
}

/** The address's query as the panel reads it: a field given twice, or without a value, is none. */
function queryAddress(query: LocationQuery): PanelAddress {
  return Object.fromEntries(Object.entries(query).flatMap(([field, value]) => typeof value === 'string' ? [[field, value]] : []))
}

/**
 * The work panel's state per conversation, for the page's lifetime.
 *
 * - While the page's address shows a conversation, the address is the one
 *   source of what its panel shows (`@demicodes/web-ui/agent/panel-address`):
 *   the state is read from it, never written beside it. A move of the panel
 *   is a move of the address, a step of the web browser's history unless it
 *   only changes how the same place is seen or corrects the address in
 *   place; a move that joins one on its way keeps it a step, and a kind's
 *   default, such as the view a file shows in, is written in the same move.
 * - A show of a tab the first read of the tabs brings happened before the
 *   page read them, and never overrides an address that names what the
 *   panel shows.
 * - What the address showed is kept per conversation: whether the panel is
 *   open and what it selected, in the account's local preferences; its
 *   pinned tabs' data, in memory. An address without `panel` that the panel
 *   did not write, such as one the sidebar opens, shows the panel as kept,
 *   and the address then says so; a phone, where the panel covers the
 *   conversation, comes up without it.
 * - The backend's tabs are read when their revision rises, with this page's
 *   changes shown at once over them.
 */
export const useWorkPanel = defineStore('work-panel', () => {
  const resources = useResources()
  const product = useProduct()
  const conversations = useConversations()
  const pluginStates = usePluginStates()
  const enabled = (plugin: string) => pluginEnabled(product.snapshot, plugin)
  const states = reactive(new Map<string, WorkState>())
  /** Each conversation's tabs; reactive on their own, so they stand beside the states, not in them. */
  const panels = new Map<string, PanelTabs>()
  /** Each conversation's pinned tabs' data as last shown, for the page's lifetime. */
  const keptPinned = reactive(new Map<string, PinnedTabs>())
  const phone = usePhone()
  /** The conversation the page shows, as the frame tells it (`shown`). */
  let shownId: string | null = null
  /**
   * The panel revision of the shown conversation the page last read, null
   * until it reads one after the conversation came on screen: a show the
   * page reads past it came while the user looked at that conversation.
   */
  let seenRevision: number | null = null
  /** The router whose address holds the panel (`attach`). */
  let router: Router | null = null
  /**
   * The address that says what the panel shows: the one the router reached,
   * or the one a move of the panel is taking it to. Null while the page
   * shows no conversation.
   */
  const address = shallowRef<ShownAddress | null>(null)
  /** The move of the router on its way, which a later move joins: a step stays a step. */
  let moving: { how: 'push' | 'replace' } | null = null

  /** The kinds of the pages the user has on. */
  function kinds() {
    return PLUGIN_PAGES.filter((page) => enabled(page.plugin)).flatMap((page) => page.kinds ?? [])
  }

  /** The conversation's tabs, made the first time they are asked for. */
  function tabsOf(conversationId: string): PanelTabs {
    let tabs = panels.get(conversationId)
    if (!tabs) {
      const created = new PanelTabs(panelBackend(conversationId), (error) => {
        reportError(t('failures.changeWorkPanel'), error, { userVisible: true })
      }, (held) => {
        for (const { plugin, revision, state } of held) {
          pluginStates.hold(plugin, conversationId, revision, state)
        }
      })
      panels.set(conversationId, created)
      let readBefore = false
      // A tab whose kind asks that the user see it opens the panel, open or
      // closed; the address then names what the page now knows of the tabs.
      watch(() => created.tabs.value, (current) => {
        const firstRead = !readBefore
        readBefore = created.read
        applyShows(conversationId, created.revision, current, firstRead)
        settle(conversationId)
      })
      tabs = created
    }
    return tabs
  }

  /** The address while it shows the conversation. */
  function addressOf(conversationId: string): ShownAddress | null {
    return address.value?.conversation === conversationId ? address.value : null
  }

  /** What the router's current history entry says of itself. */
  function entryState() {
    const parsed = entryStateSchema.safeParse(router?.options.history.state)
    const state = parsed.success ? parsed.data : {}
    return {
      marked: state[ENTRY_MARK] === true,
      steps: state[ENTRY_STEPS] ?? null,
      back: state.back ?? null,
      forward: state.forward ?? null,
    }
  }

  /** The address `route` shows, or null for a page that is no conversation. */
  function routeAddress(route: RouteLocationNormalized): ShownAddress | null {
    const conversationId = routeConversation(route)
    if (conversationId === null) {
      return null
    }
    return { conversation: conversationId, path: route.path, address: queryAddress(route.query), marked: entryState().marked }
  }

  /**
   * Whether the page knows what the conversation's panel has: the pages the
   * user has on, and the tabs once its record has them. Until then an
   * address naming what the page does not know yet stays as it is.
   */
  function ready(conversationId: string): boolean {
    return product.snapshot !== null && (!recorded(conversationId) || tabsOf(conversationId).read)
  }

  /** The selection the conversation's address names, while the panel has it or may have it once read. */
  function namedSelection(conversationId: string): string | null {
    const shown = addressOf(conversationId)
    const selection = shown ? addressSelection(shown.address) : null
    if (selection === null) {
      return null
    }
    const known = kinds().some((kind) => kind.pinned && kind.kind === selection)
      || tabsOf(conversationId).tabs.value.some((tab) => tab.id === selection)
    return known || !ready(conversationId) ? selection : null
  }

  /** Keeps whether the panel is open, as the desktop shows the conversation again; a phone keeps nothing. */
  function keepOpen(conversationId: string, open: boolean): void {
    if (phone.value || resources.local.workPanelOpen?.[conversationId] === open) {
      return
    }
    resources.local.workPanelOpen ??= {}
    resources.local.workPanelOpen[conversationId] = open
  }

  /** Keeps what the panel selected, the newest last. */
  function keepHistory(conversationId: string, history: readonly string[]): void {
    const kept = resources.local.workPanelHistory?.[conversationId] ?? []
    if (kept.length === history.length && kept.every((id, index) => id === history[index])) {
      return
    }
    resources.local.workPanelHistory ??= {}
    if (history.length === 0) {
      delete resources.local.workPanelHistory[conversationId]
    } else {
      resources.local.workPanelHistory[conversationId] = [...history]
    }
  }

  /** The conversation's panel as its view's state says now. */
  function viewOf(conversationId: string): PanelView {
    const state = stateFor(conversationId)
    return { open: state.open, history: state.history, pinned: state.pinned }
  }

  /**
   * Keeps what the address shows, for when it shows the conversation again
   * without saying so, and has the address say what the panel shows where it
   * names what the panel does not have or did not say what it shows.
   */
  function settle(conversationId: string): void {
    if (!addressOf(conversationId)) {
      return
    }
    move(conversationId, viewOf(conversationId), 'correct')
  }

  /**
   * The panel moved to `next`: kept, and, while the address shows the
   * conversation, written into it as a step of the history (`push`), in
   * its place (`replace`), or in its place once the page can judge what the
   * address names (`correct`).
   */
  function move(conversationId: string, next: PanelView, how: 'push' | 'replace' | 'correct'): void {
    keepHistory(conversationId, next.history)
    if (keptPinned.get(conversationId) !== next.pinned) {
      keptPinned.set(conversationId, next.pinned)
    }
    keepOpen(conversationId, next.open)
    const shown = addressOf(conversationId)
    if (!router || !shown) {
      return
    }
    const view = { panel: { history: next.history, tabs: tabsOf(conversationId).tabs.value }, pinned: next.pinned }
    const written = panelAddress(next.open, view, kinds())
    if (shown.marked && sameAddress(written, shown.address)) {
      return
    }
    if (how === 'correct' && !ready(conversationId)) {
      return
    }
    navigate(shown, written, how === 'push' ? 'push' : 'replace')
  }

  /**
   * The router's move to `written` from `shown`. A move on its way that this
   * one replaces was a step, so this one is too. On a phone, closing the
   * panel goes back to the entry before it opened, as its Back to
   * Conversation says.
   */
  function navigate(shown: ShownAddress, written: PanelAddress, asked: 'push' | 'replace'): void {
    const target = router
    if (!target) {
      return
    }
    const entry = entryState()
    const opens = addressSelection(written) !== null
    if (phone.value && !opens && asked === 'push' && moving === null && entry.steps !== null) {
      target.go(-entry.steps)
      return
    }
    const how: 'push' | 'replace' = moving?.how === 'push' || (asked === 'push' && shown.marked) ? 'push' : 'replace'
    const current = target.currentRoute.value
    const wasOpen = routeConversation(current) === shown.conversation && addressSelection(queryAddress(current.query)) !== null
    let steps: number | null = null
    if (opens && how === 'push') {
      steps = !wasOpen ? 1 : entry.steps === null ? null : entry.steps + 1
    } else if (opens) {
      steps = entry.steps
    }
    const token = { how }
    moving = token
    address.value = { conversation: shown.conversation, path: shown.path, address: written, marked: true }
    const location = { path: shown.path, query: { ...written }, state: { [ENTRY_MARK]: true, [ENTRY_STEPS]: steps }, force: true }
    const going = how === 'push' ? target.push(location) : target.replace(location)
    // A move nothing joined that the router did not make leaves the address
    // where the router is. An error a guard throws stays the router's, which
    // reports it as any navigation's.
    void going.then((failure) => {
      if (moving === token) {
        moving = null
        if (failure) {
          address.value = routeAddress(target.currentRoute.value)
        }
      }
    })
  }

  /** What the entry `direction` of the router's shows of the conversation's panel: open, closed, or another page. */
  function neighbour(conversationId: string, direction: HistoryDirection): 'open' | 'closed' | null {
    const at = entryState()[direction]
    if (!router || !addressOf(conversationId) || !at) {
      return null
    }
    const route = router.resolve(at)
    if (routeConversation(route) !== conversationId) {
      return null
    }
    return addressSelection(queryAddress(route.query)) === null ? 'closed' : 'open'
  }

  /**
   * Opens the panel on each tab whose kind counts more showings than this
   * page applied, once, and records the counts beside the selection history.
   *
   * - The shows the first read of the tabs brings happened before the page
   *   read them: an address that names what the panel shows stands over
   *   them.
   * - On a phone only a show read while the user looks at the conversation
   *   opens it; one that came while the user was elsewhere, read before the
   *   conversation came on screen or by the first read after, is selected in
   *   the strip with the panel left closed.
   *
   * `revision` is the panel's that `tabs` came with.
   */
  function applyShows(conversationId: string, revision: number, tabs: readonly PanelTab[], firstRead: boolean): void {
    const looking = conversationId === shownId
    const whileLooking = looking && seenRevision !== null && revision > seenRevision
    if (looking && revision >= 0) {
      seenRevision = Math.max(seenRevision ?? -1, revision)
    }
    const applied = resources.local.workPanelShown?.[conversationId] ?? {}
    const pending = pendingShows(tabs, PLUGIN_PAGES, enabled, applied)
    if (!pending) {
      return
    }
    resources.local.workPanelShown ??= {}
    resources.local.workPanelShown[conversationId] = { ...pending.applied }
    const shown = addressOf(conversationId)
    if (firstRead && shown && addressSelection(shown.address) !== null) {
      return
    }
    const view = viewOf(conversationId)
    const history = pending.shown.reduce<readonly string[]>((selected, id) => selectTab(selected, id), view.history)
    move(conversationId, { ...view, history, open: view.open || !phone.value || whileLooking }, 'push')
  }

  /**
   * The frame shows `conversationId` now, or no conversation: only what its
   * panel shows from here on opens it on a phone.
   */
  function shown(conversationId: string | null): void {
    shownId = conversationId
    const revision = conversationId === null ? -1 : (panels.get(conversationId)?.revision ?? -1)
    seenRevision = revision >= 0 ? revision : null
  }

  /**
   * The conversation's panel as it shows: while the address shows the
   * conversation, what the address says over what was kept; otherwise what
   * was kept.
   */
  function stateFor(conversationId: string): WorkState {
    if (!states.has(conversationId)) {
      const tabs = tabsOf(conversationId)
      states.set(conversationId, {
        get open(): boolean {
          const shownNow = addressOf(conversationId)
          const selection = shownNow ? addressSelection(shownNow.address) : null
          if (shownNow && (shownNow.marked || selection !== null)) {
            return selection !== null
          }
          return !phone.value && (resources.local.workPanelOpen?.[conversationId] ?? false)
        },
        get history(): readonly string[] {
          const kept = resources.local.workPanelHistory?.[conversationId] ?? []
          const selection = namedSelection(conversationId)
          return selection === null || kept.at(-1) === selection ? kept : selectTab(kept, selection)
        },
        get pinned(): PinnedTabs {
          const kept = keptPinned.get(conversationId) ?? {}
          const shownNow = addressOf(conversationId)
          const selection = namedSelection(conversationId)
          const kind = kinds().find((candidate) => candidate.pinned && candidate.kind === selection)
          if (!shownNow || !kind?.place) {
            return kept
          }
          const current = pinnedData(kept, kind)
          const data = placeData(kind, shownNow.address, current)
          return data === current ? kept : { ...kept, [kind.kind]: data }
        },
        get panel(): PanelState {
          return { history: this.history, tabs: tabs.tabs.value }
        },
        get tabsRead(): boolean {
          return tabs.read
        },
      })
    }
    // The map's own (reactive) view of the entry, never the plain object it was made from.
    return states.get(conversationId)!
  }

  // A panel a page has read is read again when its conversation's metadata says it changed.
  watch(
    () => conversations.items.map((item) => [item.id, item.metadata.shown.value?.panelRevision] as const),
    (followed) => {
      for (const [id, revision] of followed) {
        if (revision !== undefined) {
          panels.get(id)?.noticed(revision)
        }
      }
    },
  )
  // The pages the user has on say what an address names: a change of them reads the address again.
  watch(
    () => product.snapshot === null ? null : kinds().map((kind) => kind.kind).join(),
    () => {
      if (address.value) {
        settle(address.value.conversation)
      }
    },
  )

  /**
   * Holds the panel in `next`'s address: each navigation the router ends on
   * a conversation shows that conversation's panel as its address says.
   * Returns what lets go of it.
   */
  function attach(next: Router): () => void {
    router = next
    const stop = next.afterEach((to, _from, failure) => {
      if (failure) {
        return
      }
      moving = null
      address.value = routeAddress(to)
      if (address.value) {
        settle(address.value.conversation)
      }
    })
    return () => {
      stop()
      router = null
      address.value = null
      moving = null
    }
  }

  /** Whether the web browser's Back or Forward stays in the conversation's work panel. */
  function canGo(conversationId: string, direction: HistoryDirection): boolean {
    const shownNow = addressOf(conversationId)
    return shownNow !== null && addressSelection(shownNow.address) !== null && neighbour(conversationId, direction) === 'open'
  }

  /** The web browser's Back or Forward, as the panel's address bar takes it. */
  function go(direction: HistoryDirection): void {
    if (direction === 'back') {
      router?.back()
    } else {
      router?.forward()
    }
  }

  /**
   * Whether the conversation has its backend record, which its first send
   * creates. Before it, the panel binds no page and reads or sends nothing.
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

  /** Opens or closes the conversation's panel, a step of the history. */
  function setOpen(conversationId: string, open: boolean): void {
    move(conversationId, { ...viewOf(conversationId), open }, 'push')
  }

  function select(conversationId: string, id: string): void {
    const view = viewOf(conversationId)
    move(conversationId, { ...view, history: selectTab(view.history, id) }, 'push')
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
    const id = crypto.randomUUID()
    tabsOf(conversationId).change({ type: 'create', tab: { id, kind, data }, index: options.index })
    if (options.select) {
      const view = viewOf(conversationId)
      move(conversationId, { ...view, open: true, history: selectTab(view.history, id) }, 'push')
    }
    return id
  }

  /** Moves the tab `id` to `index` among the others, where the user dragged it. */
  function moveTab(conversationId: string, id: string, index: number): void {
    tabsOf(conversationId).change({ type: 'move', id, index })
  }

  /** The tab's next `data`, from its kind: only what changed is sent. */
  function update(conversationId: string, id: string, data: unknown): void {
    updatePanelTab(tabsOf(conversationId), id, data)
  }

  /**
   * Closes the tabs `ids`; the panel shows what was selected before a closed
   * one, and the address says so in its place.
   */
  function closeTabs(conversationId: string, ids: string[]): void {
    const view = { ...viewOf(conversationId) }
    closePanelTabs(view, tabsOf(conversationId), ids)
    move(conversationId, view, 'replace')
  }

  /** A pinned tab's data, as its kind replaces it: a move to another place unless `how` says it is none. */
  function updatePinned(conversationId: string, kind: string, data: unknown, how?: TabUpdate): void {
    const view = viewOf(conversationId)
    move(conversationId, { ...view, pinned: { ...view.pinned, [kind]: data } }, how?.replace ? 'replace' : 'push')
  }

  /**
   * Opens an intent in the conversation's panel (`packages/web-ui/src/plugins/intents.ts`): the
   * kind that opens it shows it, in its pinned tab or a new one, which takes the selection, with
   * the panel opened for it. What the panel already shows stays as it is (`openIntent`).
   */
  function openIn(conversationId: string, request: IntentRequest): void {
    const state = stateFor(conversationId)
    const opened = openIntent(state, PLUGIN_PAGES, enabled, request)
    if (!opened) {
      return
    }
    if (opened.created) {
      tabsOf(conversationId).change({ type: 'create', tab: opened.created })
    }
    move(conversationId, { open: true, history: selectTab(state.history, opened.selection), pinned: opened.pinned }, 'push')
  }

  /** Whether any page the user has on opens `intent`. */
  function canOpen(intent: IntentName): boolean {
    return intentKind(PLUGIN_PAGES, enabled, intent) !== null
  }

  return {
    stateFor,
    recorded,
    shown,
    attach,
    setOpen,
    load,
    select,
    add,
    move: moveTab,
    update,
    closeTabs,
    updatePinned,
    openIn,
    canGo,
    canOpen,
    go,
  }
})

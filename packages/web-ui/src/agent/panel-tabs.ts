import { activeTab, closeTabs } from './tab-close'
import type { PanelTabKind } from './panel-kinds/kind'
import { intentKind, type AnyPluginPage, type PanelKind, type PanelSession } from '../plugins/page'
import type { IntentRequest } from '../plugins/intents'

/**
 * A tab of the work panel: a fact, `{ id, kind, data }`. What its content is
 * doing is never written here, and `data` means something only to the tab's
 * kind.
 */
export interface PanelTab {
  id: string
  kind: string
  data: unknown
}

/**
 * What the work panel shows (`web-application.md` § Work panel): what the
 * page selected, and the tabs in their order, which the backend keeps
 * (`web-api.md` § Work panel state).
 */
export interface PanelState {
  /** The tabs' and pinned kinds' ids the page selected, the newest last (`selectTab`). */
  history: readonly string[]
  tabs: PanelTab[]
}

/**
 * The pinned tabs' data, by kind, as the page keeps it in memory
 * (`web-application.md` § Work panel): a kind that has none yet shows its
 * first data.
 */
export type PinnedTabs = Readonly<Record<string, unknown>>

/** The data the pinned tab of `kind` shows. */
export function pinnedData(pinned: PinnedTabs, kind: PanelTabKind): unknown {
  return Object.hasOwn(pinned, kind.kind) ? pinned[kind.kind] : kind.pinned?.data()
}

/**
 * What the panel shows: the newest selection that names a pinned kind or a
 * tab the page shows, so a closed tab gives way to the one selected before
 * it, else its first pinned tab, else its first tab, else nothing.
 */
export function shownSelection(state: PanelState, kinds: readonly PanelTabKind[]): string | null {
  const pinned = kinds.filter((kind) => kind.pinned).map((kind) => kind.kind)
  return activeTab([...pinned, ...state.tabs.map((tab) => tab.id)], state.history)
}

/**
 * What opening `intent` does (`plugin-pages.md` § Intents): the first kind of
 * a page the user has on that opens it shows the data it returns, in its
 * pinned tab, whose data `pinned` then holds, or in a tab it creates; either
 * takes the selection. Null when no such kind opens the intent.
 */
export function openIntent(
  pinned: PinnedTabs,
  pages: readonly AnyPluginPage[],
  enabled: (plugin: string) => boolean,
  request: IntentRequest,
): { selection: string; pinned: PinnedTabs; created: PanelTab | null } | null {
  const kind = intentKind(pages, enabled, request.intent)
  if (!kind) {
    return null
  }
  if (!kind.pinned) {
    const created = { id: crypto.randomUUID(), kind: kind.kind, data: opened(kind, request, null) }
    return { selection: created.id, pinned, created }
  }
  const current = Object.hasOwn(pinned, kind.kind) ? kind.schema.safeParse(pinned[kind.kind]) : null
  return {
    selection: kind.kind,
    pinned: { ...pinned, [kind.kind]: opened(kind, request, current?.success ? current.data : null) },
    created: null,
  }
}

/** The data `kind`'s tab shows for `request`, from what it shows now. */
function opened(kind: PanelKind<unknown, PanelSession | undefined>, request: IntentRequest, current: unknown): unknown {
  switch (request.intent) {
    case 'file':
      return kind.intents?.file?.(request.payload, current)
    case 'edit':
      return kind.intents?.edit?.(request.payload, current)
  }
}

/** The tab the panel shows, or null while it shows a pinned tab or nothing. */
export function selectedTab(state: PanelState, kinds: readonly PanelTabKind[]): PanelTab | null {
  const shown = shownSelection(state, kinds)
  return state.tabs.find((tab) => tab.id === shown) ?? null
}

/** The state after `ids` close: they leave the tabs and the history, so the panel shows what was selected before. */
export function removeTabs(state: PanelState, ids: readonly string[]): PanelState {
  return closeTabs(state.tabs, state.history, ids)
}

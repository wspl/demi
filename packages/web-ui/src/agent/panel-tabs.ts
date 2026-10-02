import { closeTabs } from './tab-close'
import type { PanelTabKind } from './panel-kinds/kind'
import type { PluginPage } from '../plugins/slots'
import { intentTarget } from '../plugins/slots'
import type { IntentName, IntentPayloads } from '../plugins/intents'

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
 * The work panel's state (`web-application.md` § Work panel): a selection
 * and the user's tabs, in their order. The host saves it as the backend's
 * work panel document (`web-api.md` § Work panel state).
 */
export interface PanelState {
  /** A tab's id or a pinned kind's id; null names nothing. */
  selection: string | null
  tabs: PanelTab[]
}

export function emptyPanelState(): PanelState {
  return { selection: null, tabs: [] }
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
 * What the panel shows: the selection when it names a pinned kind or a tab
 * the page shows, else its first pinned tab, else its first tab, else
 * nothing.
 */
export function shownSelection(state: PanelState, kinds: readonly PanelTabKind[]): string | null {
  const pinned = kinds.filter((kind) => kind.pinned).map((kind) => kind.kind)
  const names = state.selection
  if (names !== null && (pinned.includes(names) || state.tabs.some((tab) => tab.id === names))) {
    return names
  }
  return pinned[0] ?? state.tabs[0]?.id ?? null
}

/**
 * The panel after `intent` opened: the target's pinned tab takes the data the
 * page's target gives it and the selection; null when no page the user has on
 * opens the intent.
 */
export function openIntent<Name extends IntentName>(
  panel: { state: PanelState; pinned: PinnedTabs },
  pages: readonly PluginPage[],
  enabled: (plugin: string) => boolean,
  intent: Name,
  payload: IntentPayloads[Name],
): { state: PanelState; pinned: PinnedTabs } | null {
  const target = intentTarget(pages, enabled, intent)
  if (!target) {
    return null
  }
  const current = Object.hasOwn(panel.pinned, target.kind) ? panel.pinned[target.kind] : null
  return {
    state: { ...panel.state, selection: target.kind },
    pinned: { ...panel.pinned, [target.kind]: target.open(payload, current) },
  }
}

/** The selected tab, or null while a pinned tab or a tab that is gone is selected. */
export function selectedTab(state: PanelState): PanelTab | null {
  return state.tabs.find((tab) => tab.id === state.selection) ?? null
}

/** A new tab after the others; `select` gives it the selection. */
export function addTab(
  state: PanelState,
  tab: { kind: string; data: unknown },
  options: { select: boolean },
): { state: PanelState; id: string } {
  const id = crypto.randomUUID()
  return {
    id,
    state: {
      selection: options.select ? id : state.selection,
      tabs: [...state.tabs, { id, kind: tab.kind, data: tab.data }],
    },
  }
}

/** The tab's `data`, replaced by its kind. */
export function updateTab(state: PanelState, id: string, data: unknown): PanelState {
  return {
    ...state,
    tabs: state.tabs.map((tab) => (tab.id === id ? { ...tab, data } : tab)),
  }
}

/**
 * The state after `ids` close. A closed selection passes to the nearest
 * remaining tab before it, then the first remaining tab, then to nothing,
 * which shows the first pinned tab; another selection stays.
 */
export function removeTabs(state: PanelState, ids: readonly string[]): PanelState {
  const closing = state.selection !== null && ids.includes(state.selection)
  const next = closeTabs(state.tabs, closing ? state.selection : null, ids)
  return {
    selection: closing ? next.activeId : state.selection,
    tabs: next.tabs,
  }
}

/** The tab moved before `beforeId`, or to the end when that is null. */
export function moveTab(state: PanelState, id: string, beforeId: string | null): PanelState {
  const moving = state.tabs.find((tab) => tab.id === id)
  if (!moving || id === beforeId) {
    return state
  }
  const others = state.tabs.filter((tab) => tab.id !== id)
  const index = beforeId === null ? -1 : others.findIndex((tab) => tab.id === beforeId)
  const at = index < 0 ? others.length : index
  return { ...state, tabs: [...others.slice(0, at), moving, ...others.slice(at)] }
}

export function selectInPanel(state: PanelState, selection: string | null): PanelState {
  return { ...state, selection }
}

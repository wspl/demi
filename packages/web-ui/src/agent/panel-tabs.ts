import { activeTab, closeTabs } from './tab-close'
import { dataChanges } from './panel-changes'
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
export function pinnedData(pinned: PinnedTabs, kind: Pick<PanelTabKind, 'kind' | 'pinned'>): unknown {
  return Object.hasOwn(pinned, kind.kind) ? pinned[kind.kind] : kind.pinned?.data()
}

/**
 * What the panel shows: the newest selection that names a pinned kind or a
 * tab the page shows, so a closed tab gives way to the one selected before
 * it, else its first pinned tab, else its first tab, else nothing.
 */
export function shownSelection(state: PanelState, kinds: readonly Pick<PanelTabKind, 'kind' | 'pinned'>[]): string | null {
  const pinned = kinds.filter((kind) => kind.pinned).map((kind) => kind.kind)
  return activeTab([...pinned, ...state.tabs.map((tab) => tab.id)], state.history)
}

/** A conversation's work panel as a page shows it: whether it is open, its tabs and selection, and its pinned tabs' data. */
export interface ShownPanel {
  open: boolean
  panel: PanelState
  pinned: PinnedTabs
}

/**
 * What opening an intent does: close the panel, or open it on the kind's
 * tab, which takes the selection, its data `pinned` then holds for a
 * pinned kind, or a tab `created` for it.
 */
export type IntentOutcome =
  | { action: 'close' }
  | { action: 'open'; selection: string; pinned: PinnedTabs; created: PanelTab | null }

/**
 * What opening `request` does (`plugin-pages.md` § Intents), from a
 * control's click when `clicks`, the click's `detail`, is given. The first
 * kind of a page the user has on that opens it shows the data it returns,
 * in its pinned tab or in a tab it creates. But when the panel is open and
 * its selected tab is that kind's, showing what the kind would show for the
 * request already, the click closes the panel instead
 * (`web-application.md` § Work panel), unless it is the second click of a
 * double-click. An open no click asks for never closes. Null when no kind
 * opens the intent.
 */
export function openIntent(
  shown: ShownPanel,
  pages: readonly AnyPluginPage[],
  enabled: (plugin: string) => boolean,
  request: IntentRequest,
  clicks?: number,
): IntentOutcome | null {
  const kind = intentKind(pages, enabled, request.intent)
  if (!kind) {
    return null
  }
  if (clicks !== undefined && clicks < 2 && alreadyShows(shown, pages, enabled, kind, request)) {
    return { action: 'close' }
  }
  const { pinned } = shown
  if (!kind.pinned) {
    const created = { id: crypto.randomUUID(), kind: kind.kind, data: opened(kind, request, null) }
    return { action: 'open', selection: created.id, pinned, created }
  }
  const current = Object.hasOwn(pinned, kind.kind) ? kind.schema.safeParse(pinned[kind.kind]) : null
  return {
    action: 'open',
    selection: kind.kind,
    pinned: { ...pinned, [kind.kind]: opened(kind, request, current?.success ? current.data : null) },
    created: null,
  }
}

/**
 * Whether the open panel's selected tab is `kind`'s and already shows what
 * `request` names: the kind, which alone knows what its data means, would
 * show the same data for it.
 */
function alreadyShows(
  shown: ShownPanel,
  pages: readonly AnyPluginPage[],
  enabled: (plugin: string) => boolean,
  kind: PanelKind<unknown, PanelSession | undefined>,
  request: IntentRequest,
): boolean {
  if (!shown.open) {
    return false
  }
  const kinds = pages.filter((page) => enabled(page.plugin)).flatMap((page) => page.kinds ?? [])
  const selection = shownSelection(shown.panel, kinds)
  const data = kind.pinned
    ? (selection === kind.kind ? pinnedData(shown.pinned, kind) : undefined)
    : shown.panel.tabs.find((tab) => tab.id === selection && tab.kind === kind.kind)?.data
  const current = data === undefined ? null : kind.schema.safeParse(data)
  if (!current?.success) {
    return false
  }
  const next = opened(kind, request, current.data)
  return next !== null && typeof next === 'object' && Object.keys(dataChanges(current.data, { ...next })).length === 0
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

/**
 * Where a tab of `kind` with `data` goes among `tabs` as it is added: right
 * after the tab its kind says opened it (`PanelKind.openedBy`), behind the
 * tabs that tab opened before that still stand right after it, as a web
 * browser places the tabs a link opens; after the others (undefined) when
 * its kind names no opener or the panel no longer has that tab. The browser
 * plugin places the tabs a page opens by the same rule on the backend
 * (`live-view.md` § A browser tab in the panel).
 */
export function openedTabIndex(
  tabs: readonly PanelTab[],
  pages: readonly AnyPluginPage[],
  kind: string,
  data: unknown,
): number | undefined {
  const kinds = pages.flatMap((page) => page.kinds ?? [])
  const openerOf = (tab: Pick<PanelTab, 'kind' | 'data'>) => {
    const found = kinds.find((candidate) => candidate.kind === tab.kind)
    const parsed = found?.openedBy ? found.schema.safeParse(tab.data) : null
    return found?.openedBy && parsed?.success ? found.openedBy(parsed.data) : undefined
  }
  const opener = openerOf({ kind, data })
  const at = opener === undefined ? -1 : tabs.findIndex((tab) => tab.id === opener)
  if (at < 0) {
    return undefined
  }
  let end = at + 1
  while (end < tabs.length && openerOf(tabs[end]!) === opener) {
    end += 1
  }
  return end
}

/** The state after `ids` close: they leave the tabs and the history, so the panel shows what was selected before. */
export function removeTabs(state: PanelState, ids: readonly string[]): PanelState {
  return closeTabs(state.tabs, state.history, ids)
}

/**
 * Each tab's highest count of the times something asked that the user see
 * it, as the page last applied it, by tab id (`web-application.md` § Work
 * panel); it stands beside the page's selection history.
 */
export type AppliedShows = Readonly<Record<string, number>>

/**
 * The tabs whose kind counts more showings than the page applied
 * (`plugin-pages.md` § Work panel kinds), in the panel's order, and the
 * record once they are applied: the panel opens and selects each, so the
 * last of them shows. Tabs the panel no longer has leave the record. Null
 * when no tab asks; the record then stays as it is.
 */
export function pendingShows(
  tabs: readonly PanelTab[],
  pages: readonly AnyPluginPage[],
  enabled: (plugin: string) => boolean,
  applied: AppliedShows,
): { shown: string[]; applied: AppliedShows } | null {
  const kinds = pages.filter((page) => enabled(page.plugin)).flatMap((page) => page.kinds ?? [])
  const counts = tabs.flatMap((tab) => {
    const kind = kinds.find((candidate) => candidate.kind === tab.kind)
    const data = kind?.shows ? kind.schema.safeParse(tab.data) : null
    return kind?.shows && data?.success ? [[tab.id, kind.shows(data.data)] as const] : []
  })
  const shown = counts.filter(([id, count]) => count > (applied[id] ?? 0)).map(([id]) => id)
  if (shown.length === 0) {
    return null
  }
  const present = new Set(tabs.map((tab) => tab.id))
  const kept = Object.entries(applied).filter(([id]) => present.has(id))
  const raised = counts.filter(([id]) => shown.includes(id))
  return { shown, applied: { ...Object.fromEntries(kept), ...Object.fromEntries(raised) } }
}

/**
 * The tabs and pinned kinds whose contents the panel keeps on the page
 * (`web-application.md` § Work panel, Contents stay): each one once shown,
 * in the order first shown, until it leaves the panel (`present` lists the
 * ids it has). The selection joins at the end.
 */
export function keptContents(
  kept: readonly string[],
  selection: string | null,
  present: readonly string[],
): readonly string[] {
  const staying = kept.filter((id) => present.includes(id))
  if (selection !== null && present.includes(selection) && !staying.includes(selection)) {
    return [...staying, selection]
  }
  return staying.length === kept.length ? kept : staying
}

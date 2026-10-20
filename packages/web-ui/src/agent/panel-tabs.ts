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
 * What the work panel shows: what the page selected, and the tabs in their
 * order, which the backend keeps.
 */
export interface PanelState {
  /** The tabs' and pinned kinds' ids the page selected, the newest last (`selectTab`). */
  history: readonly string[]
  tabs: PanelTab[]
}

/**
 * The pinned tabs' data, by kind, as the page keeps it in memory: a kind that
 * has none yet shows its first data.
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

/** A conversation's work panel as a page shows it: its tabs and selection, and its pinned tabs' data. */
export interface ShownPanel {
  panel: PanelState
  pinned: PinnedTabs
}

/**
 * What opening an intent does: the panel opens on the kind's tab, which
 * takes the selection, its data `pinned` then holds for a pinned kind, or a
 * tab `created` for it.
 */
export interface IntentOutcome {
  selection: string
  pinned: PinnedTabs
  created: PanelTab | null
}

/**
 * What opening `request` does: the first kind
 * of a page the user has on that opens it shows the data it returns, in its
 * pinned tab or in a tab it creates. When the panel's selected tab is that
 * kind's and the kind, which alone knows what its data means, would show
 * the same data for the request, the tab stays as it is, its data the same
 * value, so a second click on a pill or a link leaves the panel showing it. A
 * kind that keys its tabs selects the tab whose data has the request's key,
 * where the strip holds one, as a second click on an attachment's card shows
 * its tab again. Null when no kind opens the intent.
 */
export function openIntent(
  shown: ShownPanel,
  pages: readonly AnyPluginPage[],
  enabled: (plugin: string) => boolean,
  request: IntentRequest,
): IntentOutcome | null {
  const kind = intentKind(pages, enabled, request.intent)
  if (!kind) {
    return null
  }
  const { pinned } = shown
  const kinds = pages.filter((page) => enabled(page.plugin)).flatMap((page) => page.kinds ?? [])
  const selection = shownSelection(shown.panel, kinds)
  if (!kind.pinned) {
    const tab = shown.panel.tabs.find((candidate) => candidate.id === selection && candidate.kind === kind.kind)
    const parsed = tab ? kind.schema.safeParse(tab.data) : null
    if (tab && parsed?.success && sameData(parsed.data, opened(kind, request, parsed.data))) {
      return { selection: tab.id, pinned, created: null }
    }
    const data = opened(kind, request, null)
    const same = keyedTab(kind, shown.panel.tabs, data)
    if (same) {
      return { selection: same.id, pinned, created: null }
    }
    const created = { id: crypto.randomUUID(), kind: kind.kind, data }
    return { selection: created.id, pinned, created }
  }
  const stored = Object.hasOwn(pinned, kind.kind) ? kind.schema.safeParse(pinned[kind.kind]) : null
  const current = stored?.success ? stored.data : null
  const data = opened(kind, request, current)
  if (selection === kind.kind && current !== null && sameData(current, data)) {
    return { selection, pinned, created: null }
  }
  return { selection: kind.kind, pinned: { ...pinned, [kind.kind]: data }, created: null }
}

/** The tab of `kind` among `tabs` whose data has the key of `data`, when the kind keys its tabs. */
function keyedTab(kind: PanelKind<unknown, PanelSession | undefined>, tabs: readonly PanelTab[], data: unknown): PanelTab | null {
  const { key } = kind
  if (!key) {
    return null
  }
  const wanted = key(data)
  return tabs.find((tab) => {
    if (tab.kind !== kind.kind) {
      return false
    }
    const parsed = kind.schema.safeParse(tab.data)
    return parsed.success && key(parsed.data) === wanted
  }) ?? null
}

/** Whether a kind's data `next` changes nothing of `current`, field by field. */
function sameData(current: unknown, next: unknown): boolean {
  return next !== null && typeof next === 'object' && Object.keys(dataChanges(current, { ...next })).length === 0
}

/** The data `kind`'s tab shows for `request`, from what it shows now. */
function opened(kind: PanelKind<unknown, PanelSession | undefined>, request: IntentRequest, current: unknown): unknown {
  switch (request.intent) {
    case 'file':
      return kind.intents?.file?.(request.payload, current)
    case 'edit':
      return kind.intents?.edit?.(request.payload, current)
    case 'commit':
      return kind.intents?.commit?.(request.payload, current)
    case 'web':
      return kind.intents?.web?.(request.payload, current)
    case 'attachment':
      return kind.intents?.attachment?.(request.payload, current)
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

/**
 * Each tab's highest count of the times something asked that the user see
 * it, as the page last applied it, by tab id; it stands beside the page's
 * selection history.
 */
export type AppliedShows = Readonly<Record<string, number>>

/**
 * The tabs whose kind counts more showings than the page applied, in the panel's order, and the
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
 * The tabs and pinned kinds whose contents the panel keeps on the page: each
 * one once shown, in the order first shown, until it leaves the panel
 * (`present` lists the ids it has). The selection joins at the end.
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

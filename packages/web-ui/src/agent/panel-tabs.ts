import { z } from 'zod'
import { closeTabs } from './tab-close'

/**
 * The work panel's saved state (`web-application.md` § Work panel): a
 * selection and the user's tabs. A tab is a fact, `{ id, kind, data }`; what
 * its content is doing is never written here, and `data` means something only
 * to the tab's kind.
 */
export const panelTabSchema = z.object({
  id: z.string().min(1),
  kind: z.string().min(1),
  data: z.unknown(),
})

/** The fixed views compete with the tabs for the selection; they are not tabs. */
export const FIXED_VIEWS = ['change', 'file'] as const
export type FixedView = (typeof FIXED_VIEWS)[number]

export const PANEL_TABS_MAX = 64

export const panelStateSchema = z.object({
  /** A fixed view or a tab's id. */
  selection: z.string().min(1),
  tabs: z.array(panelTabSchema).max(PANEL_TABS_MAX),
})

export type PanelTab = z.infer<typeof panelTabSchema>
export type PanelState = z.infer<typeof panelStateSchema>

export function emptyPanelState(): PanelState {
  return { selection: 'change', tabs: [] }
}

export function isFixedView(selection: string): selection is FixedView {
  return FIXED_VIEWS.some((view) => view === selection)
}

/** The selected tab, or null while a fixed view or a tab that is gone is selected. */
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
 * remaining tab before it, then the first remaining tab, then Change.
 */
export function removeTabs(state: PanelState, ids: readonly string[]): PanelState {
  const selection = isFixedView(state.selection) ? null : state.selection
  const next = closeTabs(state.tabs, selection, ids)
  return {
    selection: selection === null ? state.selection : (next.activeId ?? 'change'),
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

export function selectInPanel(state: PanelState, selection: string): PanelState {
  return { ...state, selection }
}

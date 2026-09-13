import { baseName } from '../files/paths'

/**
 * One tab in the work panel: a file to read, or the conversation's changes
 * (its diff), of which there is at most one. The panel shows tabs for the
 * conversation on screen; the host keeps them per conversation and swaps
 * the set when the conversation changes.
 */
export type WorkTab =
  | {
      id: string
      kind: 'file'
      /** The working-tree path the tab shows. */
      path: string
    }
  | {
      id: string
      kind: 'change'
    }

/** What the tab is labelled with: the file's name, or Change. */
export function workTabTitle(tab: WorkTab): string {
  return tab.kind === 'file' ? baseName(tab.path) : 'Change'
}

/** The one change tab, when it is open. */
export function changeWorkTab(tabs: readonly WorkTab[]): WorkTab | null {
  return tabs.find((tab) => tab.kind === 'change') ?? null
}

/**
 * The tabs and active tab after `ids` close. When the active tab goes, the
 * nearest remaining tab before it takes over, else the first that remains.
 */
export function closeWorkTabs(
  tabs: readonly WorkTab[],
  activeId: string | null,
  ids: readonly string[],
): { tabs: WorkTab[]; activeId: string | null } {
  const closing = new Set(ids)
  const remaining = tabs.filter((tab) => !closing.has(tab.id))
  if (activeId !== null && !closing.has(activeId)) {
    return { tabs: remaining, activeId }
  }
  const activeIndex = tabs.findIndex((tab) => tab.id === activeId)
  const before = tabs
    .slice(0, Math.max(0, activeIndex))
    .reverse()
    .find((tab) => !closing.has(tab.id))
  const next = before ?? remaining[0] ?? null
  return { tabs: remaining, activeId: next?.id ?? null }
}

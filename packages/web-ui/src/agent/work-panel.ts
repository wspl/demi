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
      /** The paths shown before, latest last; Back returns to them. */
      back: string[]
      /** The paths left by Back, latest last; Forward returns to them. */
      forward: string[]
    }
  | {
      id: string
      kind: 'change'
    }

/** A file tab showing `path`, with nothing to go back or forward to. */
export function fileWorkTab(id: string, path: string): WorkTab {
  return { id, kind: 'file', path, back: [], forward: [] }
}

/**
 * Shows `path` in the active file tab, in place: the tab turns into that
 * file and remembers the one it showed. A tab already showing the file is
 * activated instead, so a file has one tab. Without an active file tab
 * (a change tab, or none) a new file tab opens.
 */
export function showFileInTab(
  tabs: readonly WorkTab[],
  activeId: string | null,
  path: string,
  newId: () => string,
): { tabs: WorkTab[]; activeId: string | null } {
  const shown = tabs.find((tab) => tab.kind === 'file' && tab.path === path)
  if (shown) {
    return { tabs: [...tabs], activeId: shown.id }
  }
  const active = tabs.find((tab) => tab.id === activeId)
  if (active?.kind !== 'file') {
    const tab = fileWorkTab(newId(), path)
    return { tabs: [...tabs, tab], activeId: tab.id }
  }
  const replaced: WorkTab = { ...active, path, back: [...active.back, active.path], forward: [] }
  return { tabs: tabs.map((tab) => (tab.id === active.id ? replaced : tab)), activeId: active.id }
}

/** The tab showing the file it showed before, the current one kept for Forward. */
export function goBackInTab(tabs: readonly WorkTab[], id: string): WorkTab[] {
  return tabs.map((tab) => {
    if (tab.id !== id || tab.kind !== 'file' || tab.back.length === 0) {
      return tab
    }
    const back = tab.back.slice(0, -1)
    return { ...tab, path: tab.back[tab.back.length - 1]!, back, forward: [...tab.forward, tab.path] }
  })
}

/** The tab showing the file Back left, the current one kept for Back. */
export function goForwardInTab(tabs: readonly WorkTab[], id: string): WorkTab[] {
  return tabs.map((tab) => {
    if (tab.id !== id || tab.kind !== 'file' || tab.forward.length === 0) {
      return tab
    }
    const forward = tab.forward.slice(0, -1)
    return { ...tab, path: tab.forward[tab.forward.length - 1]!, forward, back: [...tab.back, tab.path] }
  })
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

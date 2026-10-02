/** Which tabs a close command in a tab's menu takes: the tab, the others, or one side of it. */
export type TabCloseScope = 'self' | 'others' | 'right' | 'left'

/** The ids `scope` closes, in strip order; empty when there is nothing on that side. */
export function tabsToClose(
  tabs: readonly { id: string }[],
  id: string,
  scope: TabCloseScope,
): string[] {
  const index = tabs.findIndex((tab) => tab.id === id)
  if (index < 0) {
    return []
  }
  switch (scope) {
    case 'self':
      return [id]
    case 'others':
      return tabs.filter((tab) => tab.id !== id).map((tab) => tab.id)
    case 'right':
      return tabs.slice(index + 1).map((tab) => tab.id)
    case 'left':
      return tabs.slice(0, index).map((tab) => tab.id)
  }
}

/**
 * The most selections a tab history keeps: more than a work panel holds tabs
 * (64) and pinned tabs, so every tab still there is in it.
 */
export const TAB_HISTORY = 100

/** `history` with `id` its newest entry: each tab once, the newest last. */
export function selectTab(history: readonly string[], id: string): string[] {
  return [...history.filter((entry) => entry !== id), id].slice(-TAB_HISTORY)
}

/**
 * The active tab of `ids`: the newest entry of `history` still among them,
 * so closing the active tab activates the one selected before it, and the
 * one before that if it closed too; the first of `ids` when none is left.
 */
export function activeTab(ids: readonly string[], history: readonly string[]): string | null {
  const present = new Set(ids)
  return history.findLast((id) => present.has(id)) ?? ids[0] ?? null
}

/** The tabs and their history after `ids` close: a closed tab leaves both. */
export function closeTabs<T extends { id: string }>(
  tabs: readonly T[],
  history: readonly string[],
  ids: readonly string[],
): { tabs: T[]; history: string[] } {
  const closing = new Set(ids)
  return {
    tabs: tabs.filter((tab) => !closing.has(tab.id)),
    history: history.filter((id) => !closing.has(id)),
  }
}

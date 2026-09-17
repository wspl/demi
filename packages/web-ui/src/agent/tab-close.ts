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
 * The tabs and active tab after `ids` close. When the active tab goes, the
 * nearest remaining tab before it takes over, else the first that remains.
 */
export function closeTabs<T extends { id: string }>(
  tabs: readonly T[],
  activeId: string | null,
  ids: readonly string[],
): { tabs: T[]; activeId: string | null } {
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

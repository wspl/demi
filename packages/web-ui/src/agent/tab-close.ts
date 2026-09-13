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

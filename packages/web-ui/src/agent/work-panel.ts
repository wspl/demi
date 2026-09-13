import { baseName } from '../files/paths'

/**
 * One tab in the work panel: a file to read, or the diff of a file.
 * The panel shows tabs for the conversation on screen; the host keeps
 * them per conversation and swaps the set when the conversation changes.
 */
export interface WorkTab {
  id: string
  kind: 'file' | 'diff'
  /** The working-tree path the tab is about. */
  path: string
}

/** What the tab is labelled with: the file's name. */
export function workTabTitle(tab: WorkTab): string {
  return baseName(tab.path)
}

/** The tab to show after `closedId` goes: the one before it, else the first. */
export function nextActiveWorkTab(tabs: readonly WorkTab[], closedId: string): string | null {
  const index = tabs.findIndex((tab) => tab.id === closedId)
  const remaining = tabs.filter((tab) => tab.id !== closedId)
  if (remaining.length === 0) {
    return null
  }
  const before = remaining[Math.max(0, index - 1)]
  return (before ?? remaining[0]!).id
}

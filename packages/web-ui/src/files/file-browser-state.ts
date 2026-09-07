/** The file browser's pure state: what is shown in which order, and where Back and Forward go. */
import type { FileBrowserEntry } from './types'
import { isHiddenName } from './paths'

export type FileBrowserSortKey = 'name' | 'modifiedAt' | 'size'

export interface FileBrowserSort {
  key: FileBrowserSortKey
  direction: 'asc' | 'desc'
}

/** Directories first, then the chosen column; names compare naturally so `file2` precedes `file10`. */
export function sortEntries(entries: readonly FileBrowserEntry[], sort: FileBrowserSort): FileBrowserEntry[] {
  const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: 'base' })
  const sign = sort.direction === 'asc' ? 1 : -1
  return [...entries].sort((a, b) => {
    if (a.isDirectory !== b.isDirectory) return a.isDirectory ? -1 : 1
    let order = 0
    if (sort.key === 'modifiedAt') order = (a.modifiedAt ?? '').localeCompare(b.modifiedAt ?? '')
    else if (sort.key === 'size') order = (a.size ?? -1) - (b.size ?? -1)
    if (order === 0) order = collator.compare(a.name, b.name) * (sort.key === 'name' ? 1 : sign)
    return order * sign
  })
}

/** The rows a query and the hidden-files switch leave visible. */
export function filterEntries(entries: readonly FileBrowserEntry[], query: string, showHidden: boolean): FileBrowserEntry[] {
  const needle = query.trim().toLowerCase()
  return entries.filter((entry) => (showHidden || !isHiddenName(entry.name)) && (!needle || entry.name.toLowerCase().includes(needle)))
}

/** Clicking the same header again flips the direction; another column starts ascending, except time, which starts newest first. */
export function nextSort(current: FileBrowserSort, key: FileBrowserSortKey): FileBrowserSort {
  if (current.key === key) return { key, direction: current.direction === 'asc' ? 'desc' : 'asc' }
  return { key, direction: key === 'modifiedAt' ? 'desc' : 'asc' }
}

/** A browser-style history over visited directories. Navigating discards the forward stack. */
export interface FileBrowserHistory {
  readonly current: string
  readonly canBack: boolean
  readonly canForward: boolean
  push(path: string): void
  back(): string | null
  forward(): string | null
  /** Replaces the whole history, for a new source. */
  reset(path: string): void
}

export function createFileBrowserHistory(initial: string): FileBrowserHistory {
  let entries = [initial]
  let index = 0
  return {
    get current() {
      return entries[index]!
    },
    get canBack() {
      return index > 0
    },
    get canForward() {
      return index < entries.length - 1
    },
    push(path) {
      if (path === entries[index]) return
      entries = [...entries.slice(0, index + 1), path]
      index = entries.length - 1
    },
    back() {
      if (index === 0) return null
      index -= 1
      return entries[index]!
    },
    forward() {
      if (index >= entries.length - 1) return null
      index += 1
      return entries[index]!
    },
    reset(path) {
      entries = [path]
      index = 0
    },
  }
}

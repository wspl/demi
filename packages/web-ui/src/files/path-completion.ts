/**
 * Completion for a field that takes a Host path: the text before the caret
 * names a directory and a query, the directory's entries that match the query
 * become a menu, and accepting one writes its name in place of the query.
 * Gallery: Files › Address Bar, and the New Project dialog's Directory.
 */
import { computed, ref, watch, type ComputedRef } from 'vue'
import fuzzysort from 'fuzzysort'
import { compareFileNames } from './file-browser-state'
import { isHiddenName, normalizePath, resolveHostPath } from './paths'
import { useShowing } from './showing'
import type { FileBrowserEntry, FileBrowserSource } from './types'

/** What a field offers: directories only, as a project's folder, or files too, as an address bar. */
export type PathCompletionKind = 'directory' | 'any'

/** Where the caret stands in a path: the directory it lists and the query that filters it. */
export interface CompletionSpot {
  /** The directory whose entries are offered, absolute and normalized. */
  directory: string
  /** What follows the last slash before the caret. */
  query: string
  /** Where the query starts in the text; the caret ends it. */
  start: number
}

/** A field's text and where its caret sits, after an edit. */
export interface PathEdit {
  text: string
  caret: number
}

/** One row of the menu: an entry and the characters of its name the query matched. */
export interface Completion {
  entry: FileBrowserEntry
  indexes: readonly number[]
}

/**
 * The spot the caret stands at in `text`. The text before the caret splits at
 * its last slash: up to and including it is the directory, the rest the
 * query. A leading `~/` is `home`; a relative directory starts from `base`,
 * and without a base it offers nothing.
 */
export function completionSpot(
  text: string,
  caret: number,
  places: { home: string; base?: string },
): CompletionSpot | null {
  const before = text.slice(0, caret)
  const start = before.lastIndexOf('/') + 1
  const typed = before.slice(0, start)
  const expanded = typed.startsWith('~/') ? `${places.home}${typed.slice(1)}` : typed
  if (places.base === undefined && !expanded.startsWith('/'))
    return null
  const absolute = places.base === undefined ? expanded : resolveHostPath(places.base, expanded)
  return { directory: normalizePath(absolute), query: before.slice(start), start }
}

/**
 * The entries a query offers, best first. The match is fuzzy and ignores
 * case: `abc` finds `a_b_c.txt`. Equal matches put directories first, then
 * names in the file browser's order. A dot entry shows only to a query that
 * starts with a dot, and a `directory` field offers no files.
 */
export function rankCompletions(
  entries: readonly FileBrowserEntry[],
  query: string,
  kind: PathCompletionKind,
): Completion[] {
  const candidates = entries.filter((entry) =>
    (kind === 'any' || entry.isDirectory) &&
    (query.startsWith('.') || !isHiddenName(entry.name)))
  // Any match counts, however weak, and every one is kept: the menu scrolls.
  const results = fuzzysort.go(query, candidates, { key: 'name', threshold: 0, limit: 0 })
  return [...results]
    .sort((a, b) => {
      if (a.score !== b.score)
        return b.score - a.score
      if (a.obj.isDirectory !== b.obj.isDirectory)
        return a.obj.isDirectory ? -1 : 1
      return compareFileNames(a.obj.name, b.obj.name)
    })
    .map((result) => ({ entry: result.obj, indexes: [...result.indexes] }))
}

/**
 * The text with `entry` in place of the spot's query: a directory's name with
 * a slash, so the next level lists, a file's name alone. What follows the
 * caret stays as it was; the caret ends the name.
 */
export function completeWith(text: string, caret: number, spot: CompletionSpot, entry: FileBrowserEntry): PathEdit {
  const name = entry.isDirectory ? `${entry.name}/` : entry.name
  return {
    text: `${text.slice(0, spot.start)}${name}${text.slice(caret)}`,
    caret: spot.start + name.length,
  }
}

/** Shows the listing of `directory` while it is the one asked for; a source without listings shows none. */
function useListing(
  source: () => Partial<Pick<FileBrowserSource, 'showListing'>>,
  directory: () => string | undefined,
) {
  return useShowing(source, directory, (shown, path) => shown.showListing?.(path))
}

export interface PathCompletionOptions {
  /** Lists the directories and names the home; without `showListing` nothing is offered. */
  source: () => Pick<FileBrowserSource, 'home'> & Partial<Pick<FileBrowserSource, 'showListing'>>
  /** Where a relative path starts; without it a relative path is offered nothing. */
  base: () => string | undefined
  kind: () => PathCompletionKind
}

/** What a key did: left to the field, used by the menu, or accepted a row into the text. */
export type PathCompletionKey =
  | { kind: 'pass' }
  | { kind: 'handled' }
  | { kind: 'accept'; edit: PathEdit; entry: FileBrowserEntry }

/**
 * The menu of a path field, following its caret. The directory the caret
 * stands in shows its listing as the source keeps it, so a directory listed
 * a moment ago, by this field or any view, offers its entries at once, and
 * filtering is local. A directory that fails to list, or a query that
 * matches nothing, shows no menu.
 */
export function usePathCompletion(options: PathCompletionOptions) {
  const spot = ref<CompletionSpot | null>(null)
  const highlighted = ref(-1)
  // Escape put the menu away until the caret's spot changes.
  const dismissed = ref(false)
  const listing = useListing(options.source, () => spot.value?.directory)

  const rows = computed(() => {
    const at = spot.value
    // A directory that cannot be listed offers nothing; why is the file browser's to say.
    const entries = listing.entry.value?.value
    return at && entries ? rankCompletions(entries, at.query, options.kind()) : []
  })
  const isOpen = computed(() => !dismissed.value && rows.value.length > 0)
  // The list opens, or its rows change, with its first row selected: the one Tab takes.
  watch(
    () => rows.value.map((row) => row.entry.name).join('\0'),
    () => {
      highlighted.value = rows.value.length > 0 ? 0 : -1
    },
    // At once, so a key pressed right after the rows changed reads the new selection.
    { flush: 'sync' },
  )

  /** Follows the caret: `caret` is null while the field has no caret, a range selected or focus gone. */
  function follow(text: string, caret: number | null): void {
    const source = options.source()
    const next = caret === null ? null : completionSpot(text, caret, { home: source.home, base: options.base() })
    const current = spot.value
    if (next?.directory === current?.directory && next?.query === current?.query && next?.start === current?.start)
      return
    spot.value = next
    dismissed.value = false
  }

  /** The text with the row at `index` accepted. */
  function accept(index: number, text: string, caret: number): PathCompletionKey {
    const at = spot.value
    const row = rows.value[index]
    if (!at || !row)
      return { kind: 'pass' }
    return { kind: 'accept', edit: completeWith(text, caret, at, row.entry), entry: row.entry }
  }

  /** Selects the row at `index`, as a pointer moving over it does. */
  function highlight(index: number): void {
    if (index >= 0 && index < rows.value.length)
      highlighted.value = index
  }

  /**
   * A key pressed in the field. While the menu shows, a row is always
   * selected, the first when it opens or its rows change: the arrows move
   * the selection, Tab accepts the selected row, and Escape puts the menu
   * away. Every other key, Return among them, is the field's, and every key
   * while no menu shows, so Return submits what the field holds and Tab
   * then moves the focus as it does anywhere.
   */
  function keydown(key: string, text: string, caret: number): PathCompletionKey {
    if (!isOpen.value)
      return { kind: 'pass' }
    const count = rows.value.length
    if (key === 'ArrowDown') {
      highlighted.value = highlighted.value < count - 1 ? highlighted.value + 1 : 0
      return { kind: 'handled' }
    }
    if (key === 'ArrowUp') {
      highlighted.value = highlighted.value > 0 ? highlighted.value - 1 : count - 1
      return { kind: 'handled' }
    }
    if (key === 'Tab')
      return accept(highlighted.value, text, caret)
    if (key === 'Escape') {
      dismissed.value = true
      return { kind: 'handled' }
    }
    return { kind: 'pass' }
  }

  return { rows, highlighted, isOpen, follow, keydown, accept, highlight }
}

/**
 * Whether the directory `text` names exists, as the listing the field's menu
 * shows with the caret at the text's end tells, so the field and what reads
 * this share one listing: `/Users/zan/Projects/demi/` exists when its own
 * listing reads, and `/Users/zan/Projects/demi` when `/Users/zan/Projects`
 * lists a folder `demi`. A listing that finds its directory gone says no.
 * Null while it is not known: the listing is on its way or failed for
 * another reason, or the text is not a path the field completes.
 */
export function useDirectoryExistence(options: Pick<PathCompletionOptions, 'source' | 'base'> & { text: () => string }): ComputedRef<boolean | null> {
  const spot = computed(() => {
    const text = options.text()
    return completionSpot(text, text.length, { home: options.source().home, base: options.base() })
  })
  const listing = useListing(options.source, () => spot.value?.directory)
  return computed(() => {
    const at = spot.value
    const shown = listing.entry.value
    if (!at || !shown)
      return null
    if (shown.value === undefined)
      return shown.failure?.kind === 'not-found' ? false : null
    return at.query === '' || shown.value.some((entry) => entry.isDirectory && entry.name === at.query)
  })
}

/**
 * Completion for a field that takes a Host path: the text before the caret
 * names a directory and a query, the directory's entries that match the query
 * become a menu, and accepting one writes its name in place of the query.
 * Gallery: Files › Address Bar, and the New Project dialog's Directory.
 */
import { computed, ref, shallowReactive } from 'vue'
import { tryOnScopeDispose } from '@vueuse/core'
import fuzzysort from 'fuzzysort'
import { compareFileNames } from './file-browser-state'
import { isHiddenName, normalizePath, resolveHostPath } from './paths'
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

export interface PathCompletionOptions {
  /** Lists the directories and names the home; without `list` nothing is offered. */
  source: () => Pick<FileBrowserSource, 'home'> & Partial<Pick<FileBrowserSource, 'list'>>
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
 * The menu of a path field, following its caret. Each directory lists once
 * for the field's life (once per source), a listing the caret has left for
 * another directory is aborted, and filtering is local. A directory that
 * fails to list, or a query that matches nothing, shows no menu.
 */
export function usePathCompletion(options: PathCompletionOptions) {
  const spot = ref<CompletionSpot | null>(null)
  // Each directory's entries, or null for one that failed to list.
  const listings = shallowReactive(new Map<string, readonly FileBrowserEntry[] | null>())
  const highlighted = ref(-1)
  // Escape put the menu away until the caret's spot changes.
  const dismissed = ref(false)
  let pending: { directory: string; controller: AbortController } | null = null
  let listedFrom: unknown = null

  const rows = computed(() => {
    const at = spot.value
    const entries = at ? listings.get(at.directory) : undefined
    return at && entries ? rankCompletions(entries, at.query, options.kind()) : []
  })
  const isOpen = computed(() => !dismissed.value && rows.value.length > 0)

  function abortPending(): void {
    pending?.controller.abort()
    pending = null
  }

  async function list(directory: string): Promise<void> {
    const source = options.source()
    if (!source.list || listings.has(directory) || pending?.directory === directory)
      return
    abortPending()
    const controller = new AbortController()
    pending = { directory, controller }
    let entries: readonly FileBrowserEntry[] | null
    try {
      entries = await source.list(directory, controller.signal)
    } catch {
      // A directory that cannot be listed offers nothing; why is the file browser's to say.
      entries = null
    }
    if (controller.signal.aborted)
      return
    pending = null
    listings.set(directory, entries)
  }

  /** Follows the caret: `caret` is null while the field has no caret, a range selected or focus gone. */
  function follow(text: string, caret: number | null): void {
    const source = options.source()
    // Listings belong to the source that made them; another source lists anew.
    if (listedFrom !== source) {
      listedFrom = source
      abortPending()
      listings.clear()
    }
    const next = caret === null ? null : completionSpot(text, caret, { home: source.home, base: options.base() })
    const current = spot.value
    if (next?.directory === current?.directory && next?.query === current?.query && next?.start === current?.start)
      return
    spot.value = next
    highlighted.value = -1
    dismissed.value = false
    if (next)
      void list(next.directory)
  }

  /** The text with the row at `index` accepted. */
  function accept(index: number, text: string, caret: number): PathCompletionKey {
    const at = spot.value
    const row = rows.value[index]
    if (!at || !row)
      return { kind: 'pass' }
    return { kind: 'accept', edit: completeWith(text, caret, at, row.entry), entry: row.entry }
  }

  /**
   * A key pressed in the field. While the menu shows, the arrows move the
   * highlight, Tab accepts the highlighted row or the first, Enter the
   * highlighted one, and Escape puts the menu away; every other key, and
   * Enter with nothing highlighted, is the field's.
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
      return accept(Math.max(0, highlighted.value), text, caret)
    if (key === 'Enter' && highlighted.value >= 0)
      return accept(highlighted.value, text, caret)
    if (key === 'Escape') {
      dismissed.value = true
      return { kind: 'handled' }
    }
    return { kind: 'pass' }
  }

  tryOnScopeDispose(abortPending)

  return { rows, highlighted, isOpen, follow, keydown, accept }
}

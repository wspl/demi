import { computed, readonly, ref } from 'vue'
import { tryOnScopeDispose } from '@vueuse/core'

/**
 * Type-select, as in Finder: with a list, a tree or a menu focused, typing a
 * name moves to the first row from the top whose name starts with what was
 * typed. Gallery: Files › Rows, the file browser's list, a crumb's menu.
 */

/** A pause this long ends the query: the next key starts a new one. */
export const TYPE_SELECT_PAUSE_MS = 1000

export interface TypeSelectOptions {
  /** The names of the rows a query can reach, in the order they show, top first. */
  names: () => readonly string[]
  /** Moves the selection, or the keyboard cursor, to the row at this index of `names`. */
  select: (index: number) => void
}

/** What a query reads of a key: which one, and whether a modifier makes it a shortcut. */
export type TypeSelectKey = Pick<KeyboardEvent, 'key' | 'ctrlKey' | 'metaKey' | 'altKey'>

/** Whether `name` starts with `query`, case ignored. */
function startsWith(name: string, query: string): boolean {
  return name.slice(0, query.length).toLowerCase() === query.toLowerCase()
}

/**
 * A printable key without Ctrl, Command or Option adds to the query, and the
 * first row whose name starts with it is selected; a row that none matches
 * leaves the selection where it is. A key within the pause of the last one
 * adds to the query; the pause, or Escape, ends it. Space and Backspace add
 * and take away only while a query is live, and otherwise keep the host's
 * meaning.
 */
export function useTypeSelect(options: TypeSelectOptions) {
  const query = ref('')
  let timer: ReturnType<typeof setTimeout> | undefined

  const match = computed(() => query.value === ''
    ? -1
    : options.names().findIndex((name) => startsWith(name, query.value)))

  function end(): void {
    clearTimeout(timer)
    timer = undefined
    query.value = ''
  }

  /** Sets the query and selects its first match; an empty one ends it. */
  function type(next: string): void {
    if (next === '') {
      end()
      return
    }
    clearTimeout(timer)
    timer = setTimeout(end, TYPE_SELECT_PAUSE_MS)
    query.value = next
    if (match.value >= 0)
      options.select(match.value)
  }

  /** Reads a key; true when the query took it, and the host's own meaning must not run. */
  function keydown(event: TypeSelectKey): boolean {
    if (event.ctrlKey || event.metaKey || event.altKey)
      return false
    const live = query.value !== ''
    if (event.key === 'Escape') {
      if (!live)
        return false
      end()
      return true
    }
    if (event.key === 'Backspace') {
      if (!live)
        return false
      type([...query.value].slice(0, -1).join(''))
      return true
    }
    if (event.key === ' ' && !live)
      return false
    // A named key (`Enter`, `ArrowDown`, `Dead`) is longer than one character.
    if ([...event.key].length !== 1)
      return false
    type(query.value + event.key)
    return true
  }

  /** The positions of the live query in `name`, when `name` starts with it; null otherwise. */
  function prefix(name: string): number[] | null {
    if (query.value === '' || !startsWith(name, query.value))
      return null
    return Array.from({ length: query.value.length }, (_, index) => index)
  }

  tryOnScopeDispose(end)

  return {
    /** What has been typed; empty while no query is live. */
    query: readonly(query),
    /** Whether a row's name starts with the live query. */
    matched: computed(() => match.value >= 0),
    keydown,
    prefix,
    end,
  }
}

export type TypeSelect = ReturnType<typeof useTypeSelect>

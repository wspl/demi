import type { EditorState, Text } from '@codemirror/state'
import { getSearchQuery, searchPanelOpen, type SearchQuery } from '@codemirror/search'

/**
 * Past this many matches the find bar counts "N+" and the scrollbar marks only
 * the first N: a query such as the regular expression `.` in a large file
 * would otherwise stall every update.
 */
export const MATCH_LIMIT = 9999

export interface SearchMatches {
  /** In the text's order. */
  ranges: readonly { from: number; to: number }[]
  /** False when the query matches more than MATCH_LIMIT times. */
  complete: boolean
}

const NONE: SearchMatches = { ranges: [], complete: true }

// A query and a text never change, so the find bar and the scrollbar share
// one search of each pair.
const found = new WeakMap<SearchQuery, { doc: Text; matches: SearchMatches }>()

/**
 * The find bar's matches, up to MATCH_LIMIT. None while the bar is closed,
 * when CodeMirror stops highlighting them too.
 */
export function searchMatches(state: EditorState): SearchMatches {
  const query = getSearchQuery(state)
  if (!searchPanelOpen(state) || !query.valid)
    return NONE
  const cached = found.get(query)
  if (cached?.doc === state.doc)
    return cached.matches
  const ranges: { from: number; to: number }[] = []
  let complete = true
  const cursor = query.getCursor(state)
  for (let match = cursor.next(); !match.done; match = cursor.next()) {
    if (ranges.length === MATCH_LIMIT) {
      complete = false
      break
    }
    ranges.push({ from: match.value.from, to: match.value.to })
  }
  const matches = { ranges, complete }
  found.set(query, { doc: state.doc, matches })
  return matches
}

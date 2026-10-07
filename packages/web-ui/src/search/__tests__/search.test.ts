import { afterEach, expect, jest, test } from 'bun:test'
import { effectScope, nextTick } from 'vue'
import { SEARCH_PAUSE_MS, useConversationSearch, type SearchRow } from '../search'

// The window searches what the field holds once typing pauses, and never
// shows an answer to something it no longer holds. The clock is bun's fake
// one; about 10 ms.

function row(title: string): SearchRow {
  return { conversationId: title, title, titleRanges: [], archived: false, lastActiveAt: '2026-10-07T08:00:00.000Z', match: null }
}

let stop = () => {}
afterEach(() => {
  stop()
  jest.useRealTimers()
})

/** The window's search over a source whose answers the test settles, by query. */
function start() {
  const asked: string[] = []
  const answers = new Map<string, (rows: SearchRow[]) => void>()
  const scope = effectScope()
  const search = scope.run(() => useConversationSearch(
    () => (query) => {
      asked.push(query)
      return new Promise((resolve) => answers.set(query, resolve))
    },
    () => [row('Recent')],
  ))!
  stop = () => scope.stop()
  return { search, asked, answer: (query: string, rows: SearchRow[]) => answers.get(query)!(rows) }
}

async function settle(): Promise<void> {
  await nextTick()
  await Promise.resolve()
}

test('typing searches once it pauses, and a late answer to an older query is dropped', async () => {
  jest.useFakeTimers()
  const { search, asked, answer } = start()
  expect(search.view.value).toEqual({ kind: 'recent', rows: [row('Recent')] })

  search.query.value = 'ts'
  await settle()
  jest.advanceTimersByTime(SEARCH_PAUSE_MS - 1)
  search.query.value = 'ts2307'
  await settle()
  jest.advanceTimersByTime(SEARCH_PAUSE_MS)
  expect(asked).toEqual(['ts2307'])

  search.query.value = 'ts2307 cache'
  await settle()
  jest.advanceTimersByTime(SEARCH_PAUSE_MS)
  expect(asked).toEqual(['ts2307', 'ts2307 cache'])
  answer('ts2307 cache', [row('New')])
  answer('ts2307', [row('Old')])
  await settle()
  expect(search.view.value).toEqual({ kind: 'results', query: 'ts2307 cache', rows: [row('New')] })

  // An emptied field lists the recent conversations again.
  search.query.value = '  '
  await settle()
  expect(search.view.value.kind).toBe('recent')
})

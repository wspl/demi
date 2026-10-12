import { expect, test } from 'bun:test'
import { effectScope, nextTick, ref } from 'vue'
import { useModelSearch } from '../model-search'
import type { SettingsModelMatch } from '../types'

// The Add Model dialog's list is always the latest text's matches: a search
// that is still on its way when the text changes is cancelled, and its late
// answer is never shown.

function match(name: string): SettingsModelMatch {
  return {
    ref: `google/${name}`,
    vendor: 'google',
    model: { id: name, name, contextWindow: 1000, outputLimit: null, efforts: [], extensions: [], fastTier: null },
  }
}

/** A search that answers when the test says; each call is one entry. */
function controlledSearch() {
  const calls: { query: string; signal: AbortSignal; answer: (matches: SettingsModelMatch[]) => void; fail: (error: Error) => void }[] = []
  const waiting: (() => void)[] = []
  const search = (query: string, signal: AbortSignal) =>
    new Promise<SettingsModelMatch[]>((resolve, reject) => {
      calls.push({ query, signal, answer: resolve, fail: reject })
      waiting.splice(0).forEach((wake) => wake())
    })
  const called = (count: number) =>
    new Promise<void>((resolve) => {
      const check = () => (calls.length >= count ? resolve() : waiting.push(check))
      check()
    })
  return { calls, search, called }
}

test('typing searches the trimmed text after the pause, and an empty text has no matches', async () => {
  const query = ref('')
  const { calls, search, called } = controlledSearch()
  const scope = effectScope()
  const found = scope.run(() => useModelSearch(query, () => search, 0))!
  expect(calls).toHaveLength(0)

  query.value = '  gemini fl '
  await called(1)
  expect(calls.map((call) => call.query)).toEqual(['gemini fl'])
  calls[0]!.answer([match('Gemini Flash')])
  await nextTick()
  await Promise.resolve()
  expect(found.results.value.map((m) => m.model.name)).toEqual(['Gemini Flash'])

  query.value = ''
  await nextTick()
  expect(found.results.value).toEqual([])
  scope.stop()
})

test('a search the text outran is aborted and its late answer is never shown', async () => {
  const query = ref('')
  const { calls, search, called } = controlledSearch()
  const scope = effectScope()
  const found = scope.run(() => useModelSearch(query, () => search, 0))!

  query.value = 'gem'
  await called(1)
  query.value = 'gemini'
  await called(2)
  expect(calls[0]!.signal.aborted).toBe(true)
  expect(calls[1]!.signal.aborted).toBe(false)

  calls[1]!.answer([match('Gemini 3.8 Flash')])
  await Promise.resolve()
  await Promise.resolve()
  calls[0]!.answer([match('Gemma')])
  await Promise.resolve()
  await Promise.resolve()
  expect(found.results.value.map((m) => m.model.name)).toEqual(['Gemini 3.8 Flash'])
  scope.stop()
})

test('a failed search leaves the list empty and says so; the next text starts clean', async () => {
  const query = ref('')
  const { calls, search, called } = controlledSearch()
  const scope = effectScope()
  const found = scope.run(() => useModelSearch(query, () => search, 0))!

  query.value = 'gem'
  await called(1)
  calls[0]!.fail(new Error('offline'))
  await Promise.resolve()
  await Promise.resolve()
  expect(found.failed.value).toBe(true)
  expect(found.results.value).toEqual([])

  query.value = ''
  await nextTick()
  expect(found.failed.value).toBe(false)
  scope.stop()
})

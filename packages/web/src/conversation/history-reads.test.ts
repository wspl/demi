import { afterEach, expect, jest, test } from 'bun:test'
import { reactive } from 'vue'
import { isAbortError } from '@demicodes/utils'
import { EMPTY_TRANSCRIPT, heldBlocks } from '@demicodes/web-ui/transport/protocol'
import { createHistoryReads, type HistoryHolder } from './history-reads'

// How the page reads an agent's transcript beyond what its session sends:
// a subagent's transcript opens at any message, and a read no view needs
// any more is stopped. The backend is a stubbed fetch that answers when a
// test lets it. A few milliseconds.

const realFetch = globalThis.fetch

afterEach(() => {
  globalThis.fetch = realFetch
  jest.useRealTimers()
})

const createdAt = '2026-10-15T12:00:00.000Z'
const model = {
  providerId: 'stub', thinking: null, serviceTierId: null,
  model: { id: 'stub', name: 'Stub', contextWindow: 1000, outputLimit: null, thinking: [], acceptedExtensions: [] },
}
const text = (id: string) => ({ type: 'text' as const, id, createdAt, model, text: id })

/** A conversation whose child `reader` has a transcript of 500 blocks, of which the page holds the latest. */
function holder(): HistoryHolder {
  return reactive({
    id: 'c1',
    history: EMPTY_TRANSCRIPT,
    failures: {},
    instructions: [],
    summaries: {},
    shownAt: null,
    subagents: [{
      id: 'reader',
      history: { length: 500, version: undefined, windows: [{ start: 499, blocks: [text('b499')] }] },
      shownAt: null,
      failures: {},
    }],
  })
}

/**
 * Answers each transcript read with the page around its `around` block once
 * the test releases it, and refuses it when its request is stopped.
 */
function playBackend() {
  const asked: string[] = []
  const held: (() => void)[] = []
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const path = String(input)
    asked.push(path)
    await new Promise<void>((resolve, reject) => {
      held.push(resolve)
      init?.signal?.addEventListener('abort', () => {
        held.splice(held.indexOf(resolve), 1)
        reject(init.signal!.reason)
      }, { once: true })
    })
    const around = new URL(path, 'http://example.com').searchParams.get('around')
    const index = Number(around?.slice(1))
    return Response.json({ start: index - 1, length: 500, blocks: [text(`b${index - 1}`), text(`b${index}`), text(`b${index + 1}`)], instructions: [], summaries: [] })
  }) as typeof fetch
  return { asked, release: () => held.shift()!() }
}

function reads() {
  return createHistoryReads<HistoryHolder>(
    () => new AbortController().signal,
    () => true,
    (holder, node) => `/conversations/${holder.id}/agents/${node ?? holder.id}`,
  )
}

// A search result in an early part of a long subagent's transcript used to
// read its latest page, then one page before another until one held it.
test('a subagent’s transcript opens at a message it does not hold in one read around it', async () => {
  const backend = playBackend()
  const conversation = holder()
  const read = reads().readAround(conversation, 'reader', 'b20')
  await Promise.resolve()
  backend.release()
  await read

  expect(backend.asked).toEqual(['/api/conversations/c1/agents/reader/transcript?around=b20'])
  const reader = conversation.subagents[0]!
  expect(reader.shownAt).toBe('b20')
  expect(heldBlocks(reader.history).map((block) => block.id)).toEqual(['b19', 'b20', 'b21', 'b499'])
})

test('a read no view needs any more stops shortly after, and one asked for again meanwhile goes on', async () => {
  jest.useFakeTimers()
  const backend = playBackend()
  const history = reads()
  const conversation = holder()

  // The panel closes while the read runs, and nobody asks again.
  const stopped = history.readAround(conversation, 'reader', 'b20').catch((error: unknown) => error)
  history.release(conversation, 'reader')
  jest.advanceTimersByTime(500)
  expect(isAbortError(await stopped)).toBe(true)

  // The panel closes, and shows the same message again before the read stops.
  const kept = history.readAround(conversation, 'reader', 'b40')
  history.release(conversation, 'reader')
  const again = history.readAround(conversation, 'reader', 'b40')
  jest.advanceTimersByTime(500)
  backend.release()
  await Promise.all([kept, again])
  expect(backend.asked).toHaveLength(2)
  expect(heldBlocks(conversation.subagents[0]!.history).map((block) => block.id)).toContain('b40')
})

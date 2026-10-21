import { afterEach, expect, jest, test } from 'bun:test'
import { reactive } from 'vue'
import { isAbortError } from '@demicodes/utils'
import { EMPTY_TRANSCRIPT, heldBlocks } from '@demicodes/web-ui/transport/protocol'
import { boundedHistory } from '@demicodes/web-ui/agent/history'
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
  return { asked, held, release: () => held.shift()!() }
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

// The list shows a loading row at the top while a page is read for it, and
// not while it only could be: the window's start is not the transcript's.
test('a window says it reads a page only while the page is under way', async () => {
  const backend = playBackend()
  globalThis.fetch = (async (input: RequestInfo | URL) => {
    backend.asked.push(String(input))
    await new Promise<void>((resolve) => backend.held.push(resolve))
    return Response.json({ start: 496, length: 500, blocks: [text('b496'), text('b497'), text('b498')], instructions: [], summaries: [] })
  }) as typeof fetch
  const history = reads()
  const conversation = holder()
  expect(history.reading(conversation, 'reader')).toEqual({ before: false, after: false })

  const read = history.readBefore(conversation, 'reader')
  await Promise.resolve()
  expect(history.reading(conversation, 'reader')).toEqual({ before: true, after: false })
  expect(history.reading(conversation, 'other')).toEqual({ before: false, after: false })
  backend.release()
  await read

  expect(history.reading(conversation, 'reader')).toEqual({ before: false, after: false })
  expect(backend.asked).toEqual(['/api/conversations/c1/agents/reader/transcript?before=499&edge=b499'])
})

// A transcript of 3,000 blocks whose window the page holds from 2,000 on, its
// view not yet having told a block: the page before the window comes whole,
// whatever the window bound would keep around the latest.
test('a page read before the window stays whole while the view has told no block', async () => {
  const blocks = (from: number, to: number) => Array.from({ length: to - from }, (_, offset) => text(`b${from + offset}`))
  globalThis.fetch = (async () => Response.json({ start: 1000, length: 3000, blocks: blocks(1000, 2000), instructions: [], summaries: [] })) as unknown as typeof fetch
  const conversation = reactive<HistoryHolder>({
    id: 'c1',
    history: EMPTY_TRANSCRIPT,
    failures: {},
    instructions: [],
    summaries: {},
    shownAt: null,
    subagents: [{
      id: 'reader',
      history: { length: 3000, version: undefined, windows: [{ start: 2000, blocks: blocks(2000, 3000) }] },
      shownAt: null,
      failures: {},
    }],
  })

  await reads().readBefore(conversation, 'reader')

  const held = conversation.subagents[0]!.history
  expect(held.windows.map((window) => [window.start, window.blocks.length])).toEqual([[1000, 2000]])
})

/**
 * A subagent of 6,000 blocks whose window the page holds from 5,000 on, its
 * view at the block `b5200`, and a backend that answers each page before an
 * index with the thousand blocks that end there.
 */
function stackedReader(shownWithin: number) {
  const blocks = (from: number, to: number) => Array.from({ length: to - from }, (_, offset) => text(`b${from + offset}`))
  const asked: string[] = []
  globalThis.fetch = (async (input: RequestInfo | URL) => {
    const path = String(input)
    asked.push(path.replace(/.*transcript\?/, ''))
    const before = Number(new URL(path, 'http://example.com').searchParams.get('before'))
    const start = Math.max(0, before - 1000)
    return Response.json({ start, length: 6000, blocks: blocks(start, before), instructions: [], summaries: [] })
  }) as typeof fetch
  const conversation = reactive<HistoryHolder>({
    id: 'c1',
    history: EMPTY_TRANSCRIPT,
    failures: {},
    instructions: [],
    summaries: {},
    shownAt: null,
    subagents: [{
      id: 'reader',
      history: { length: 6000, version: undefined, windows: [{ start: 5000, blocks: blocks(5000, 6000) }] },
      shownAt: 'b5200',
      shownWithin,
      failures: {},
    }],
  })
  const starts = () => conversation.subagents[0]!.history.windows.map((window) => [window.start, window.start + window.blocks.length])
  return { conversation, asked, starts }
}

// The page above a window that folds into few rows is read while the view
// stays at its block, 200 blocks below the window's top: the next patch of the
// running subagent bounds the history again, and the page just read must stay.
test('a page read above the view is not let go by the next patch of the transcript', async () => {
  const { conversation, starts } = stackedReader(200)
  const history = reads()

  await history.readBefore(conversation, 'reader')
  expect(starts()).toEqual([[4000, 6000]])
  // What a `transcript_patch` of the subagent does to its history.
  const reader = conversation.subagents[0]!
  reader.history = boundedHistory(reader.history, reader)

  expect(starts()).toEqual([[4000, 6000]])
})

// 2,000 blocks above the reader fold into less than three screens: the view
// asks for the page above each time the page before it arrives, and tells the
// same block with the blocks that lie in its reach. Each page is read once,
// and none is let go to read the one before it again.
test('pages read one after another above a view that stays at its block are read once and kept', async () => {
  const { conversation, asked, starts } = stackedReader(200)
  const history = reads()

  await history.readBefore(conversation, 'reader')
  history.showAt(conversation, 'reader', 'b5200', 1200)
  await history.readBefore(conversation, 'reader')
  const reader = conversation.subagents[0]!
  reader.history = boundedHistory(reader.history, reader)

  expect(asked).toEqual(['before=5000&edge=b5000', 'before=4000&edge=b4000'])
  expect(starts()).toEqual([[3000, 6000]])
})

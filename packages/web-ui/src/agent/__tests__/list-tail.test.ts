import { expect, test } from 'bun:test'
import type { Block } from '@demicodes/protocol'
import { listTailBlocks } from '../list-tail'
import { createdAt, model } from './agent-harness'

const steer = { id: 'steer-1', content: [{ type: 'text' as const, text: 'also' }] }

test('a compaction in progress shows at the end of the transcript, before the steers that wait for it', () => {
  const tail = listTailBlocks({
    phase: 'compacting',
    blocks: [],
    pendingCalls: [],
    pendingSteers: [steer],
    queue: [{ id: 'queued-1', content: [{ type: 'text', text: 'next' }] }],
  })

  expect(tail.turn.map((block) => block.type)).toEqual(['compaction_progress'])
  expect(tail.waiting.map((block) => block.type)).toEqual(['pending_steer', 'queue_divider', 'queued_message'])
})

test('the divider in progress goes once the pass ends, so the finished one takes its place', () => {
  for (const phase of ['idle', 'running'] as const) {
    expect(listTailBlocks({ phase, blocks: [], pendingCalls: [], pendingSteers: [steer], queue: [] }).turn).toEqual([])
  }
})

test("the calls being written follow the transcript from their description, before the steers, and leave once the transcript holds their block", () => {
  const writing = [
    { toolUseId: 'call-1', toolName: 'shell', description: 'Write the categorizer' },
    { toolUseId: 'call-2', toolName: 'shell', description: null },
  ]
  const tail = listTailBlocks({ phase: 'running', blocks: [], pendingCalls: writing, pendingSteers: [steer], queue: [] })
  // A call without its title yet shows nothing, so no placeholder flashes.
  expect(tail.turn.map((block) => block.id)).toEqual(['pending-call:call-1'])
  expect(tail.waiting.map((block) => block.id)).toEqual(['pending-steer:steer-1'])
  // The block arrived before the list without it: the call shows once.
  const held: Block = {
    type: 'tool_call', id: 'b1', toolUseId: 'call-1', toolName: 'shell', input: '{}', status: 'executing',
    output: [], view: null, createdAt, model,
  }
  const titled = writing.map((call) => ({ ...call, description: call.description ?? 'Run the categorizer' }))
  const after = listTailBlocks({ phase: 'running', blocks: [held], pendingCalls: titled, pendingSteers: [], queue: [] })
  expect(after.turn.map((block) => block.id)).toEqual(['pending-call:call-2'])
})

// The row of a message waiting to be confirmed becomes the confirmed message:
// whichever the server tells first, the message shows once.
test('a message the transcript holds shows there alone, not again in the tail', () => {
  const written: Block = {
    type: 'user', id: 'u-2', turnId: 'm-2', createdAt, model, content: [{ type: 'text', text: 'next' }], preamble: null,
  }
  const tail = listTailBlocks({
    phase: 'running',
    blocks: [written],
    pendingCalls: [],
    pendingSteers: [],
    queue: [{ id: 'm-2', content: [{ type: 'text', text: 'next' }] }],
    pendingSubmissions: [{ id: 'm-2', way: 'queue', text: 'next', attachments: [], error: null }],
  })
  expect(tail).toEqual({ turn: [], waiting: [] })
})

// A message on its way shows as what it will be from the send
// (`product.md` § Steer or queue): the next message in the turn's flow, a
// steer after the steers, or a queued message counted under the queue's
// divider; once the session holds it, its confirmed row alone shows.
test('a message on its way stands where what it will be stands, and leaves once the session holds it', () => {
  const queued = { id: 'q-1', content: [{ type: 'text' as const, text: 'later' }] }
  const tail = (way: 'send' | 'steer' | 'queue', held: { steers?: typeof steer[]; queue?: typeof queued[] } = {}) => listTailBlocks({
    phase: 'running',
    blocks: [],
    pendingCalls: [],
    pendingSteers: held.steers ?? [steer],
    queue: held.queue ?? [queued],
    pendingSubmissions: [{ id: 'm-3', way, text: 'also', attachments: [], error: null }],
  })
  const ids = (way: 'send' | 'steer' | 'queue', held?: Parameters<typeof tail>[1]) => {
    const { turn, waiting } = tail(way, held)
    return { turn: turn.map((block) => block.id), waiting: waiting.map((block) => block.type === 'queue_divider' ? `divider:${block.count}` : block.id) }
  }
  expect(ids('send')).toEqual({ turn: ['pending-submission:m-3'], waiting: ['pending-steer:steer-1', 'divider:1', 'queued:q-1'] })
  expect(ids('steer')).toEqual({ turn: [], waiting: ['pending-steer:steer-1', 'pending-submission:m-3', 'divider:1', 'queued:q-1'] })
  expect(ids('queue')).toEqual({ turn: [], waiting: ['pending-steer:steer-1', 'divider:2', 'queued:q-1', 'pending-submission:m-3'] })
  const confirmed = { id: 'm-3', content: [{ type: 'text' as const, text: 'also' }] }
  expect(ids('steer', { steers: [steer, confirmed] })).toEqual({ turn: [], waiting: ['pending-steer:steer-1', 'pending-steer:m-3', 'divider:1', 'queued:q-1'] })
  expect(ids('queue', { queue: [queued, confirmed] })).toEqual({ turn: [], waiting: ['pending-steer:steer-1', 'divider:2', 'queued:q-1', 'queued:m-3'] })
})

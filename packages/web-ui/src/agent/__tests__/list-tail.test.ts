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

  expect(tail.map((block) => block.type)).toEqual([
    'compaction_progress',
    'pending_steer',
    'queue_divider',
    'queued_message',
  ])
})

test('the divider in progress goes once the pass ends, so the finished one takes its place', () => {
  for (const phase of ['idle', 'running'] as const) {
    expect(listTailBlocks({ phase, blocks: [], pendingCalls: [], pendingSteers: [steer], queue: [] }).map((block) => block.type))
      .toEqual(['pending_steer'])
  }
})

test("the calls being written follow the transcript, before the steers, and leave once the transcript holds their block", () => {
  const writing = [
    { toolUseId: 'call-1', toolName: 'shell_exec', description: 'Write the categorizer' },
    { toolUseId: 'call-2', toolName: 'shell_exec', description: null },
  ]
  const tail = listTailBlocks({ phase: 'running', blocks: [], pendingCalls: writing, pendingSteers: [steer], queue: [] })
  expect(tail.map((block) => block.id)).toEqual(['pending-call:call-1', 'pending-call:call-2', 'pending-steer:steer-1'])
  // The block arrived before the list without it: the call shows once.
  const held: Block = {
    type: 'tool_call', id: 'b1', toolUseId: 'call-1', toolName: 'shell_exec', input: '{}', status: 'executing',
    output: [], view: null, createdAt, model,
  }
  const after = listTailBlocks({ phase: 'running', blocks: [held], pendingCalls: writing, pendingSteers: [], queue: [] })
  expect(after.map((block) => block.id)).toEqual(['pending-call:call-2'])
})

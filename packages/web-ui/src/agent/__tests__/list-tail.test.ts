import { expect, test } from 'bun:test'
import { listTailBlocks } from '../list-tail'

const steer = { id: 'steer-1', content: [{ type: 'text' as const, text: 'also' }] }

test('a compaction in progress shows at the end of the transcript, before the steers that wait for it', () => {
  const tail = listTailBlocks({
    phase: 'compacting',
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
    expect(listTailBlocks({ phase, pendingSteers: [steer], queue: [] }).map((block) => block.type))
      .toEqual(['pending_steer'])
  }
})

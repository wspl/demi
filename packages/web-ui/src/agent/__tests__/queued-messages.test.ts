import { expect, test } from 'bun:test'
import { queuedMessagesToRenderBlocks } from '../queued-messages'

test('empty queue adds no render blocks', () => {
  expect(queuedMessagesToRenderBlocks([])).toEqual([])
})

test('queue renders a divider then one bubble per item', () => {
  expect(queuedMessagesToRenderBlocks([
    { id: 'q1', text: 'first' },
    { id: 'q2', content: [{ type: 'text', text: 'second' }] },
  ])).toEqual([
    { type: 'queue_divider', id: 'queue-divider', count: 2 },
    {
      type: 'queued_message',
      id: 'queued:q1',
      queueId: 'q1',
      content: [{ type: 'text', text: 'first' }],
    },
    {
      type: 'queued_message',
      id: 'queued:q2',
      queueId: 'q2',
      content: [{ type: 'text', text: 'second' }],
    },
  ])
})

import { expect, test } from 'bun:test'
import { queuedMessagesToRenderBlocks } from '../queued-messages'

test('empty queue adds no render blocks', () => {
  expect(queuedMessagesToRenderBlocks([])).toEqual([])
})

test('queue renders one bubble per item, drawn from its content', () => {
  expect(queuedMessagesToRenderBlocks([
    { id: 'q1', content: [{ type: 'text', text: 'first' }] },
    { id: 'q2', content: [{ type: 'text', text: 'second' }] },
  ])).toEqual([
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

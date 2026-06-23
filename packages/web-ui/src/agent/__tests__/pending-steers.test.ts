import { expect, test } from 'bun:test'
import { reactive } from 'vue'
import type { Block, UserContentBlock } from '@demi/core'
import {
  createPendingSteerMessage,
  pendingSteersToRenderBlocks,
  reconcilePendingSteers,
} from '../pending-steers'

test('pending steer renders as a tail block without mutating transcript blocks', () => {
  const content = text('same-turn guidance')
  const pending = createPendingSteerMessage('local-1', content, [])

  const renderBlocks = pendingSteersToRenderBlocks([pending])

  expect(renderBlocks).toEqual([
    {
      type: 'pending_steer',
      id: 'pending-steer:local-1',
      pendingSteerId: 'local-1',
      content,
    },
  ])
})

test('pending steer accepts queued content from reactive state', () => {
  const content = reactive(text('queued as steer')) as UserContentBlock[]
  const pending = createPendingSteerMessage('local-1', content, [])

  expect(pending.content).toEqual(text('queued as steer'))
  expect(pending.content).not.toBe(content)
})

test('reconcile keeps pending steer when only baseline steer blocks exist', () => {
  const baseline = [steerBlock('existing', 'same text')]
  const pending = createPendingSteerMessage('local-1', text('same text'), baseline)

  expect(reconcilePendingSteers(baseline, [pending])).toEqual([pending])
})

test('reconcile removes pending steer after a matching transcript steer materializes', () => {
  const baseline = [steerBlock('existing', 'old steer')]
  const pending = createPendingSteerMessage('local-1', text('same-turn guidance'), baseline)
  const nextBlocks = [...baseline, steerBlock('materialized', 'same-turn guidance')]

  expect(reconcilePendingSteers(nextBlocks, [pending])).toEqual([])
})

test('reconcile removes pending steer when reference resolution changes materialized content', () => {
  const pending = createPendingSteerMessage('local-1', [{ type: 'reference', reference: '@file' }], [])

  expect(reconcilePendingSteers([steerBlock('materialized', 'resolved file content')], [pending])).toEqual([])
})

test('reconcile handles duplicate pending steer messages one materialized block at a time', () => {
  const first = createPendingSteerMessage('local-1', text('duplicate'), [])
  const second = createPendingSteerMessage('local-2', text('duplicate'), [])

  expect(reconcilePendingSteers([steerBlock('materialized-1', 'duplicate')], [first, second])).toEqual([second])
  expect(reconcilePendingSteers([steerBlock('materialized-1', 'duplicate'), steerBlock('materialized-2', 'duplicate')], [first, second])).toEqual([])
})

function text(value: string): UserContentBlock[] {
  return [{ type: 'text', text: value }]
}

function steerBlock(id: string, value: string): Block {
  return {
    type: 'steer',
    id,
    turnId: 'turn-1',
    createdAt: '2026-06-23T00:00:00.000Z',
    model: null,
    content: text(value),
  } as unknown as Block
}

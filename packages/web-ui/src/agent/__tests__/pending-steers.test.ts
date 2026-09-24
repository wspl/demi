import { expect, test } from 'bun:test'
import type { UserContentBlock } from '@demicodes/protocol'
import { pendingSteersToRenderBlocks } from '../pending-steers'

test('a pending steer renders as a tail block of its own', () => {
  const content: UserContentBlock[] = [{ type: 'text', text: 'same-turn guidance' }]

  expect(pendingSteersToRenderBlocks([{ id: 'local-1', content }])).toEqual([
    {
      type: 'pending_steer',
      id: 'pending-steer:local-1',
      pendingSteerId: 'local-1',
      content,
    },
  ])
})

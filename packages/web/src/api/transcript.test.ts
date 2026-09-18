import { expect, test } from 'bun:test'
import type { Block } from '@demicodes/core'
import { blockSchema } from '@demicodes/web-ui/transport/protocol'
import { transcriptSchema } from './transcript'

const receipt: Extract<Block, { type: 'agent_message' }> = {
  type: 'agent_message', id: 'subagent:child:42', turnId: 'turn', createdAt: '2026-09-12T12:00:01.000Z',
  model: {
    providerId: 'fixture', thinking: null,
    model: { id: 'fixture', name: 'Fixture', contextWindow: 100000, outputLimit: null, inputLimit: null, thinking: [], acceptedExtensions: [] },
  },
  message: {
    id: 'subagent:child:42',
    sender: { id: 'child', description: 'UI implementation', round: 42 },
    recipientId: 'parent', timestamp: '2026-09-12T12:00:00.000Z',
    content: '**Finished** the shared component.', event: { type: 'completion', outcome: 'completed' },
  },
}

test('a cold transcript keeps agent receipts with their source metadata', () => {
  const transcript = transcriptSchema.parse({
    blocks: [receipt],
    failures: {},
    subagents: [
      {
        id: 'child', name: 'UI implementation', phase: 'completed',
        startedAt: '2026-09-12T12:00:00.000Z', endedAt: '2026-09-12T12:00:01.000Z',
        blocks: [receipt],
        failures: {},
      },
    ],
  })

  expect(transcript.blocks).toEqual([receipt])
  expect(transcript.subagents[0]?.blocks).toEqual([receipt])
})

test('the product rejects corrupt receipt identities and unsupported outcomes', () => {
  expect(blockSchema.safeParse({ ...receipt, id: 'different' }).success).toBe(false)
  expect(blockSchema.safeParse({ ...receipt, message: { ...receipt.message, sender: { ...receipt.message.sender, round: 43 } } }).success).toBe(false)
  expect(blockSchema.safeParse({ ...receipt, message: { ...receipt.message, event: { type: 'completion', outcome: 'unknown' } } }).success).toBe(false)
})

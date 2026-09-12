import { expect, test } from 'bun:test'
import type { Block } from '@demicodes/core'
import { blockSchema, transcriptSchema, decodeServerFrame } from './transcript'

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

test('cold transcript, reconnect reset, and live patches retain agent receipts with source metadata', () => {
  expect(transcriptSchema.parse({ blocks: [receipt], subagents: [] }).blocks).toEqual([receipt])
  expect(decodeServerFrame({ type: 'transcript_reset', epoch: 'fixture', revision: 1, blocks: [receipt] }))
    .toMatchObject({ blocks: [receipt] })
  expect(decodeServerFrame({ type: 'transcript_patch', revision: 2, patches: [{ op: 'add', path: ['blocks', 0], value: receipt }] }))
    .toMatchObject({ patches: [{ value: receipt }] })
})

test('the product rejects corrupt receipt identities and unsupported outcomes', () => {
  expect(blockSchema.safeParse({ ...receipt, id: 'different' }).success).toBe(false)
  expect(blockSchema.safeParse({ ...receipt, message: { ...receipt.message, sender: { ...receipt.message.sender, round: 43 } } }).success).toBe(false)
  expect(blockSchema.safeParse({ ...receipt, message: { ...receipt.message, event: { type: 'completion', outcome: 'unknown' } } }).success).toBe(false)
})

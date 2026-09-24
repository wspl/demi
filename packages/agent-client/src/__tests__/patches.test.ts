import { expect, test } from 'bun:test'
import { resolve } from 'node:path'
import { blockSchema, serverFrameSchema, type Block } from '@demicodes/protocol'
import { z } from 'zod'
import { applyTranscriptPatches } from '../patch'
import { harness, text } from './harness'

// The agent's Rust tests record a patch sequence with the transcript it must
// produce; the one applier and the client must rebuild exactly that
// transcript (`contracts.md` § One patch applier).
const fixture = resolve(import.meta.dir, '../../../../crates/agent/tests/agent/fixtures/transcript-patches.json')
const cases = z
  .array(z.strictObject({ name: z.string(), reset: z.json(), patches: z.array(z.json()), transcript: z.array(z.json()) }))
  .parse(await Bun.file(fixture).json())

test('the patches the Rust session produced rebuild its transcript', () => {
  expect(cases.length).toBeGreaterThan(0)
  for (const recorded of cases) {
    const reset = serverFrameSchema.parse(recorded.reset)
    if (reset.type !== 'transcript_reset') {
      throw new Error(`${recorded.name}: the sequence starts with ${reset.type}`)
    }
    let blocks: Block[] = reset.blocks
    for (const value of recorded.patches) {
      const frame = serverFrameSchema.parse(value)
      if (frame.type !== 'transcript_patch') {
        throw new Error(`${recorded.name}: ${frame.type} among the patches`)
      }
      blocks = applyTranscriptPatches(blocks, frame.patches)
    }
    expect(blocks).toEqual(z.array(blockSchema).parse(recorded.transcript))
  }
})

test('a client that receives the recorded frames holds the transcript, and asks for nothing', () => {
  for (const recorded of cases) {
    const h = harness()
    h.receiveValue(recorded.reset)
    for (const value of recorded.patches) {
      h.receiveValue(value)
    }
    expect(h.client.transcript().blocks).toEqual(z.array(blockSchema).parse(recorded.transcript))
    expect(h.sent).toEqual([])
    expect(h.events.map((event) => event.type)).not.toContain('disconnected')
  }
})

test('append_text replaces the block it extends instead of changing it', () => {
  const original = text('a', 'start')
  const applied = applyTranscriptPatches([original], [{ op: 'append_text', index: 0, delta: ' end' }])
  expect(applied[0]).toMatchObject({ text: 'start end' })
  expect(original).toMatchObject({ text: 'start' })
})

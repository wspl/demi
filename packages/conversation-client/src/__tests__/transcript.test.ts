import { expect, test } from 'bun:test'
import { resolve } from 'node:path'
import { blockSchema, serverFrameSchema, type Block } from '@demicodes/protocol'
import { z } from 'zod'
import {
  EMPTY_TRANSCRIPT,
  addPage,
  applyTranscriptPatches,
  heldBlocks,
  heldEdge,
  resetTranscript,
  withWholeBlock,
  type HeldTranscript,
} from '../transcript'
import { harness, text } from './harness'

// The agent's Rust tests record a patch sequence with the transcript it must
// produce; the one applier and the client must rebuild exactly that
// transcript (`contracts.md` § One patch applier).
const fixture = resolve(import.meta.dir, '../../../../crates/agent-server/tests/agent-server/fixtures/transcript-patches.json')
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
    let held = resetTranscript(EMPTY_TRANSCRIPT, reset)
    for (const value of recorded.patches) {
      const frame = serverFrameSchema.parse(value)
      if (frame.type !== 'transcript_patch') {
        throw new Error(`${recorded.name}: ${frame.type} among the patches`)
      }
      held = applyTranscriptPatches(held, frame.patches)
    }
    expect(heldBlocks(held)).toEqual(z.array(blockSchema).parse(recorded.transcript))
  }
})

test('a client that receives the recorded frames tells the page the transcript, and asks for nothing', () => {
  for (const recorded of cases) {
    const h = harness()
    h.receiveValue(recorded.reset)
    for (const value of recorded.patches) {
      h.receiveValue(value)
    }
    expect(h.blocks()).toEqual(z.array(blockSchema).parse(recorded.transcript))
    expect(h.sent).toEqual([])
    expect(h.events.map((event) => event.type)).not.toContain('disconnected')
  }
})

test('append_text replaces the block it extends instead of changing it', () => {
  const original = text('a', 'start')
  const held = applyTranscriptPatches(
    { length: 1, windows: [{ start: 0, blocks: [original] }] },
    [{ op: 'append_text', index: 0, delta: ' end' }],
  )
  expect(heldBlocks(held)[0]).toMatchObject({ text: 'start end' })
  expect(original).toMatchObject({ text: 'start' })
})

/** Ten blocks, `b0` to `b9`, of which the page holds 2–3 and 7–9. */
function twoWindows(): HeldTranscript {
  return {
    length: 10,
    windows: [
      { start: 2, blocks: [text('b2', '2'), text('b3', '3')] },
      { start: 7, blocks: [text('b7', '7'), text('b8', '8'), text('b9', '9')] },
    ],
  }
}

/** Each window as its start and its blocks' ids. */
function shape(held: HeldTranscript): [number, string[]][] {
  return held.windows.map((window) => [window.start, window.blocks.map((block) => block.id)])
}

test('patches change the blocks the page holds and move the indices after them', () => {
  const cases: { name: string, patch: Parameters<typeof applyTranscriptPatches>[1][number], length: number, windows: [number, string[]][] }[] = [
    { name: 'an add in a gap joins no window', patch: { op: 'add', index: 5, value: text('n', 'new') }, length: 11, windows: [[2, ['b2', 'b3']], [8, ['b7', 'b8', 'b9']]] },
    { name: 'an add inside a window', patch: { op: 'add', index: 3, value: text('n', 'new') }, length: 11, windows: [[2, ['b2', 'n', 'b3']], [8, ['b7', 'b8', 'b9']]] },
    { name: 'an add at the end joins the latest window', patch: { op: 'add', index: 10, value: text('n', 'new') }, length: 11, windows: [[2, ['b2', 'b3']], [7, ['b7', 'b8', 'b9', 'n']]] },
    { name: 'an add before every window', patch: { op: 'add', index: 0, value: text('n', 'new') }, length: 11, windows: [[3, ['b2', 'b3']], [8, ['b7', 'b8', 'b9']]] },
    { name: 'a truncate inside a window', patch: { op: 'truncate', length: 8 }, length: 8, windows: [[2, ['b2', 'b3']], [7, ['b7']]] },
    { name: 'a truncate before a window drops it', patch: { op: 'truncate', length: 3 }, length: 3, windows: [[2, ['b2']]] },
    { name: 'a block the page does not hold is not replaced', patch: { op: 'replace_block', index: 5, value: text('n', 'new') }, length: 10, windows: [[2, ['b2', 'b3']], [7, ['b7', 'b8', 'b9']]] },
  ]
  for (const { name, patch, length, windows } of cases) {
    const held = applyTranscriptPatches(twoWindows(), [patch])
    expect([held.length, shape(held)], name).toEqual([length, windows])
  }
})

test('a page joins the windows it meets, and a block the page holds stays as it is', () => {
  const live = withWholeBlock(twoWindows(), text('b3', 'streamed'))
  const page = { start: 3, length: 9, blocks: [text('b3', 'stored'), text('b4', '4'), text('b5', '5'), text('b6', '6')] }
  const held = addPage(live, page, true)
  expect(shape(held)).toEqual([[2, ['b2', 'b3', 'b4', 'b5', 'b6', 'b7', 'b8', 'b9']]])
  // The stream's length stands; the page's block 3 does not replace the held one.
  expect(held.length).toBe(10)
  expect(heldBlocks(held)[1]).toEqual(text('b3', 'streamed'))
  expect(heldBlocks(held)[2]).toEqual({ ...text('b4', '4'), light: true })
  // Without a stream, the page's length is the transcript's.
  expect(addPage(EMPTY_TRANSCRIPT, page, false).length).toBe(9)
})

test('a reset replaces the blocks from its start, and one elsewhere than asked keeps only itself', () => {
  const reset = { start: 9, length: 11, blocks: [text('b9', 'whole'), text('b10', '10')] }
  expect(shape(resetTranscript(twoWindows(), reset, 9))).toEqual([[2, ['b2', 'b3']], [7, ['b7', 'b8', 'b9', 'b10']]])
  // The transcript was rewritten under the page: what it held no longer joins.
  const rewritten = { start: 6, length: 8, blocks: [text('r6', '6'), text('r7', '7')] }
  expect(shape(resetTranscript(twoWindows(), rewritten, 9))).toEqual([[6, ['r6', 'r7']]])
})

test('the page names its first executing call, or else its last block', () => {
  const call = (id: string, status: 'executing' | 'completed'): Block => ({
    type: 'tool_call',
    id,
    createdAt: '2026-09-24T12:00:00.000Z',
    model: (text('x', '') as Extract<Block, { type: 'text' }>).model,
    toolUseId: id,
    toolName: 'shell',
    input: '{}',
    status,
    output: [],
    view: null,
  })
  expect(heldEdge(twoWindows())).toEqual({ from: 9, edge: 'b9' })
  const running: HeldTranscript = { length: 4, windows: [{ start: 1, blocks: [call('c1', 'completed'), call('c2', 'executing'), text('t', 'x')] }] }
  expect(heldEdge(running)).toEqual({ from: 2, edge: 'c2' })
  // A page that holds no window at the end names nothing.
  expect(heldEdge({ length: 12, windows: twoWindows().windows })).toBeUndefined()
})

test('open and sync name the blocks the page holds, and the reset says what the client asked', () => {
  const h = harness()
  void h.client.open(() => ({ from: 9, edge: 'b9' }))
  expect(h.sent).toEqual([{ type: 'open', from: 9, edge: 'b9' }])
  h.receive({ type: 'opened' })
  h.receive({ type: 'transcript_reset', start: 9, length: 10, blocks: [text('b9', '9')], version: { epoch: 'e', revision: 1 } })
  expect(h.events.find((event) => event.type === 'transcript_reset')).toMatchObject({ start: 9, asked: 9 })
  h.receive({ type: 'transcript_patch', patches: [], revision: 3 })
  expect(h.sent.at(-1)).toEqual({ type: 'sync_transcript', from: 9, edge: 'b9' })
})

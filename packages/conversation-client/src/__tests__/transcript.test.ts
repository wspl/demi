import { expect, test } from 'bun:test'
import { resolve } from 'node:path'
import { blockSchema, serverFrameSchema, type Block } from '@demicodes/protocol'
import { z } from 'zod'
import {
  EMPTY_TRANSCRIPT,
  addPage,
  applyTranscriptPatches,
  boundedTranscript,
  heldBlocks,
  heldEdge,
  resetTranscript,
  withWholeBlock,
  type HeldTranscript,
} from '../transcript'
import { ROOT, harness, text } from './harness'

// The agent's Rust tests record a patch sequence with the transcript it must
// produce; the one applier and the client must rebuild exactly that
// transcript.
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

/** Twenty blocks, `b0` to `b19`, of which the page holds 2–3 and 6–19. */
function longTail(): HeldTranscript {
  const block = (index: number) => text(`b${index}`, String(index))
  return {
    length: 20,
    windows: [
      { start: 2, blocks: [block(2), block(3)] },
      { start: 6, blocks: Array.from({ length: 14 }, (_, offset) => block(6 + offset)) },
    ],
    version: { epoch: 'e', revision: 4 },
  }
}

test('a transcript shown holds the blocks around what its view shows and the latest, and lets the rest go', () => {
  const cases: { name: string, anchor: string | null, windows: [number, string[]][] }[] = [
    { name: 'at the latest, the latest blocks alone', anchor: null, windows: [[17, ['b17', 'b18', 'b19']]] },
    { name: 'in an earlier window, around it and the latest', anchor: 'b3', windows: [[2, ['b2', 'b3']], [17, ['b17', 'b18', 'b19']]] },
    { name: 'far up the latest window, a gap between', anchor: 'b8', windows: [[6, ['b6', 'b7', 'b8', 'b9', 'b10', 'b11']], [17, ['b17', 'b18', 'b19']]] },
    { name: 'near the end, one run', anchor: 'b15', windows: [[12, ['b12', 'b13', 'b14', 'b15', 'b16', 'b17', 'b18', 'b19']]] },
    { name: 'a block not held, as the latest', anchor: 'b0', windows: [[17, ['b17', 'b18', 'b19']]] },
  ]
  for (const { name, anchor, windows } of cases) {
    const held = boundedTranscript(longTail(), anchor, 3)
    // The latest blocks stay, so the stream's version still vouches for them.
    expect([held.length, shape(held), held.version], name).toEqual([20, windows, { epoch: 'e', revision: 4 }])
  }
  const within: HeldTranscript = { ...longTail(), windows: longTail().windows.slice(1) }
  expect(boundedTranscript(within, 'b10', 20)).toBe(within)
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

test('a reset replaces the blocks from its start, light but its last, and keeps what the page shows', () => {
  const reset = { start: 9, length: 11, blocks: [text('b9', 'whole'), text('b10', '10')] }
  const vouched = resetTranscript(twoWindows(), reset, 9)
  expect(shape(vouched)).toEqual([[2, ['b2', 'b3']], [7, ['b7', 'b8', 'b9', 'b10']]])
  // A row opened later reads the light ones whole; the last may still be streaming, and comes whole.
  expect(heldBlocks(vouched).map((block) => [block.id, block.light ?? false, block.stale ?? false])).toEqual([
    ['b2', false, false], ['b3', false, false], ['b7', false, false], ['b8', false, false], ['b9', true, false], ['b10', false, false],
  ])
  // The transcript was rewritten under the page, or the page was away too
  // long to name where it stopped: what it shows stays, as a hole that the
  // page reads again, and a page that arrives replaces it.
  const rewritten = { start: 6, length: 8, blocks: [text('r6', '6'), text('r7', '7')] }
  for (const asked of [9, undefined]) {
    const kept = resetTranscript(twoWindows(), rewritten, asked)
    expect(shape(kept)).toEqual([[2, ['b2', 'b3']], [6, ['r6', 'r7']]])
    expect(heldBlocks(kept).map((block) => block.stale ?? false)).toEqual([true, true, false, false])
    const read = addPage(kept, { start: 2, length: 8, blocks: [text('b2', 'two'), text('b3', 'three'), text('b4', '4'), text('b5', '5')] }, true)
    expect(shape(read)).toEqual([[2, ['b2', 'b3', 'b4', 'b5', 'r6', 'r7']]])
    expect(heldBlocks(read).slice(0, 2)).toEqual([{ ...text('b2', 'two'), light: true }, { ...text('b3', 'three'), light: true }])
  }
})

test('a block added at the end that no window reaches starts a run of its own', () => {
  const held = { length: 10, windows: [{ start: 2, blocks: [text('b2', '2'), text('b3', '3')] }] }
  const added = applyTranscriptPatches(held, [{ op: 'add', index: 10, value: text('n', 'new') }])
  expect([added.length, shape(added)]).toEqual([11, [[2, ['b2', 'b3']], [10, ['n']]]])
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
    endedAt: status === 'executing' ? null : '2026-09-24T12:00:00.000Z', output: [],
    view: null,
  })
  const version = { epoch: 'e', revision: 4 }
  expect(heldEdge({ ...twoWindows(), version })).toEqual({ version, from: 9, edge: 'b9' })
  const running: HeldTranscript = { length: 4, version, windows: [{ start: 1, blocks: [call('c1', 'completed'), call('c2', 'executing'), text('t', 'x')] }] }
  expect(heldEdge(running)).toEqual({ version, from: 2, edge: 'c2' })
  // A page that holds no window at the end, or none a stream vouched for, names nothing.
  expect(heldEdge({ length: 12, version, windows: twoWindows().windows })).toBeUndefined()
  expect(heldEdge(twoWindows())).toBeUndefined()
})

test('a subscription names the blocks the page holds, again at a gap, and the reset says what the client asked', () => {
  const h = harness({ opened: false })
  const held = { version: { epoch: 'e', revision: 1 }, from: 9, edge: 'b9' }
  void h.client.open({ root: ROOT, held: () => held })
  expect(h.sent).toEqual([{ type: 'subscribe', node: ROOT, held }, { type: 'open' }])
  h.receive({ type: 'opened' })
  h.receive({ type: 'transcript_reset', node: ROOT, start: 9, length: 10, blocks: [text('b9', '9')], version: { epoch: 'e', revision: 1 } })
  expect(h.events.find((event) => event.type === 'transcript_reset')).toMatchObject({ start: 9, asked: 9 })
  h.receive({ type: 'transcript_patch', node: ROOT, patches: [], revision: 3 })
  expect(h.sent.at(-1)).toEqual({ type: 'subscribe', node: ROOT, held })
})

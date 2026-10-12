import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, expect, spyOn, test } from 'bun:test'
import { effectScope, type EffectScope } from 'vue'
import { applyTranscriptPatches, heldBlocks, type HeldTranscript } from '@demicodes/conversation-client'
import type { Block, TranscriptPatch } from '@demicodes/protocol'
import { useStreamReveal } from '../useStreamReveal'
import { STREAM_PACE } from '../../ui/stream-reveal'
import { model } from '../../agent/__tests__/agent-harness'

// What a view that mounts on a streaming reply shows still fading, from the
// times the page's transcript received each piece: a few milliseconds, on
// happy-dom for the document the reveal listens to.

GlobalRegistrator.register({ url: 'https://demi.test/' })

const scopes: EffectScope[] = []
afterEach(() => scopes.splice(0).forEach((scope) => scope.stop()))

const reply: Block = {
  type: 'text',
  id: 'text-1',
  createdAt: '2026-10-12T10:00:00.000Z',
  model,
  text: '',
}

/** The transcript after each batch of `patches` arrived at its time. */
function received(batches: readonly [number, TranscriptPatch[]][]): HeldTranscript {
  const live: HeldTranscript = { length: 0, windows: [{ start: 0, blocks: [] }] }
  return batches.reduce((held, [at, patches]) => applyTranscriptPatches(held, patches, at), live)
}

/** The steps still fading in a reveal that mounts on the reply at `now`. */
function mountAt(held: HeldTranscript, now: number) {
  const block = heldBlocks(held)[0]!
  const clock = spyOn(performance, 'now').mockReturnValue(now)
  try {
    const text = block.type === 'text' ? block.text : ''
    const scope = effectScope()
    scopes.push(scope)
    return scope.run(() => useStreamReveal(text, true, (whole) => whole, block.arrivals))!.steps.value
  } finally {
    clock.mockRestore()
  }
}

test('a new reply\'s first piece fades from when it arrived, though the view mounts with it there', () => {
  const held = received([[1000, [{ op: 'add', index: 0, value: { ...reply, text: 'Sun' } }]]])
  expect(mountAt(held, 1012)).toEqual([{ start: 0, end: 3, at: 1000 }])
})

test('a view that mounts again mid-stream goes on with each fade where it is, never starting one over', () => {
  const held = received([
    [1000, [{ op: 'add', index: 0, value: reply }, { op: 'append_text', index: 0, delta: 'Sunlight looks white, ' }]],
    [1300, [{ op: 'append_text', index: 0, delta: 'but it is really ' }]],
    [1500, [{ op: 'append_text', index: 0, delta: 'a mix.' }]],
  ])
  // The first piece finished its fade before the view mounted, so it shows solid; the
  // later two are 240 ms and 40 ms into theirs.
  expect(1540 - 1000).toBeGreaterThan(STREAM_PACE.fadeMs)
  expect(mountAt(held, 1540)).toEqual([
    { start: 22, end: 39, at: 1300 },
    { start: 39, end: 45, at: 1500 },
  ])
})

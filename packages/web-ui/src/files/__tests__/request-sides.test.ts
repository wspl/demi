import { expect, test } from 'bun:test'
import { effectScope, nextTick, shallowRef } from 'vue'
import type { EditCopies } from '@demicodes/protocol'
import type { ChangeSides } from '../changes'
import { useRequestSides } from '../request-sides'

// Cost: reactivity and promises only, under a millisecond.

/** What the transcript names for the view as one frame derives it: a new object each time. */
interface Frame {
  selection: string | null
  copies: EditCopies | null
}

test('a frame of the turn naming the same sides keeps the diff shown, and a new end of it replaces it without a loading state', async () => {
  const reads: { copies: EditCopies; answer(sides: ChangeSides): void }[] = []
  const read = (copies: EditCopies) => new Promise<ChangeSides | null>((answer) => reads.push({ copies, answer }))
  const frame = shallowRef<Frame>({ selection: 'src/app.ts:all', copies: { original: 'blob-1', modified: 'blob-2' } })
  const scope = effectScope()
  const sides = scope.run(() => useRequestSides(() => frame.value.selection, () => frame.value.copies, () => read))!
  expect(sides.state.value.phase).toBe('loading')
  reads[0]!.answer({ original: 'one', modified: 'two' })
  await nextTick()
  const shown = sides.state.value
  expect(shown).toEqual({ phase: 'ready', sides: { original: 'one', modified: 'two' } })

  // Every streamed frame derives the transcript anew: the same names in new objects.
  for (let turn = 0; turn < 3; turn += 1) {
    frame.value = { selection: 'src/app.ts:all', copies: { original: 'blob-1', modified: 'blob-2' } }
    await nextTick()
  }
  expect(reads).toHaveLength(1)
  expect(sides.state.value).toBe(shown)

  // The agent edits the file again: All Changes ends at a new blob.
  frame.value = { selection: 'src/app.ts:all', copies: { original: 'blob-1', modified: 'blob-3' } }
  await nextTick()
  expect(reads).toHaveLength(2)
  expect(sides.state.value).toBe(shown)
  reads[1]!.answer({ original: 'one', modified: 'three' })
  await nextTick()
  expect(sides.state.value).toEqual({ phase: 'ready', sides: { original: 'one', modified: 'three' } })

  // Another edit chosen is other content, read as such.
  frame.value = { selection: 'src/app.ts:0', copies: { original: 'blob-1', modified: 'blob-2' } }
  await nextTick()
  expect(sides.state.value.phase).toBe('loading')
  scope.stop()
})

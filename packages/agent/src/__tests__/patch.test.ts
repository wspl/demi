import { expect, test } from 'bun:test'
import type { Block, ModelSelection } from '@demicodes/core'
import { Transcript, applyTranscriptPatches } from '../index'

const model: ModelSelection = {
  providerId: 'stub',
  model: {
    id: 'test-model',
    name: 'Test Model',
    contextWindow: 100_000,
    inputLimit: null,
    thinking: [],
    acceptedExtensions: [],
  },
  thinking: null,
}

test('journal patches replicate appended blocks and streaming text deltas', () => {
  const transcript = new Transcript()
  const replica: Block[] = []

  transcript.pushUserTurn('turn-1', model, [{ type: 'text', text: 'hello' }])
  transcript.applyProviderEvent(model, { type: 'text_delta', text: 'Hi ' })
  transcript.applyProviderEvent(model, { type: 'text_delta', text: 'there' })

  const drained = transcript.takePatches()
  expect(drained).not.toBeNull()
  expect(drained!.revision).toBe(1)
  // Consecutive deltas to the same block coalesce into the add (first delta
  // creates the block) plus one append_text.
  expect(drained!.patches.map((patch) => patch.op)).toEqual(['add', 'add', 'append_text'])

  const applied = applyTranscriptPatches(replica, drained!.patches)
  expect(applied).toEqual(transcript.snapshot().blocks)
})

test('journal patches replicate tool completion as a block replace', () => {
  const transcript = new Transcript()
  transcript.pushUserTurn('turn-1', model, [{ type: 'text', text: 'run' }])
  transcript.applyProviderEvent(model, {
    type: 'tool_call_requested',
    toolUseId: 'call-1',
    toolName: 'shell_exec',
    input: { script: 'ls' },
  })
  let replica = applyTranscriptPatches([], transcript.takePatches()!.patches)

  transcript.completeToolCall('call-1', [{ type: 'text', text: 'done' }], false, { result: 'ok' })
  const drained = transcript.takePatches()!
  expect(drained.revision).toBe(2)
  expect(drained.patches.map((patch) => patch.op)).toEqual(['replace_block'])

  replica = applyTranscriptPatches(replica, drained.patches)
  expect(replica).toEqual(transcript.snapshot().blocks)
  expect(replica[1]).toMatchObject({ type: 'tool_call', status: 'completed' })
})

test('journal add values are snapshots, unaffected by later mutation of the live block', () => {
  const transcript = new Transcript()
  transcript.applyProviderEvent(model, { type: 'text_delta', text: 'first' })
  const drained = transcript.takePatches()!
  // Mutate the live block after draining; the drained patch must not change.
  transcript.applyProviderEvent(model, { type: 'text_delta', text: ' second' })

  const applied = applyTranscriptPatches([], drained.patches)
  expect(applied[0]).toMatchObject({ type: 'text', text: 'first' })
})

test('rewind for retry emits a full replace and drops finer-grained history', () => {
  const transcript = new Transcript()
  transcript.pushUserTurn('turn-1', model, [{ type: 'text', text: 'try' }])
  transcript.pushSteer('turn-1', model, [{ type: 'text', text: 'also this' }])
  transcript.applyProviderEvent(model, { type: 'text_delta', text: 'partial answer' })
  const replica = applyTranscriptPatches([], transcript.takePatches()!.patches)

  const userBlock = transcript.rewindToLastUserTurn()
  expect(userBlock?.turnId).toBe('turn-1')

  const drained = transcript.takePatches()!
  expect(drained.patches.map((patch) => patch.op)).toEqual(['replace'])
  const applied = applyTranscriptPatches(replica, drained.patches)
  expect(applied.map((block) => block.type)).toEqual(['user', 'steer'])
  expect(applied).toEqual(transcript.snapshot().blocks)
})

test('takePatches returns null when nothing changed and revisions stay monotonic', () => {
  const transcript = new Transcript()
  expect(transcript.takePatches()).toBeNull()
  transcript.pushUserTurn('turn-1', model, [{ type: 'text', text: 'a' }])
  expect(transcript.takePatches()!.revision).toBe(1)
  expect(transcript.takePatches()).toBeNull()
  transcript.pushAbort(model)
  expect(transcript.takePatches()!.revision).toBe(2)
})

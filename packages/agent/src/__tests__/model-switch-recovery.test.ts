import { expect, test } from 'bun:test'
import { deferred } from '@demicodes/utils'
import type { AgentProvider, InferenceRequest, ProviderEvent } from '@demicodes/provider'
import { events } from '@demicodes/provider/testing'
import { createRuntime, createSession, makeTranscript, model, RecordingProvider, text } from './helpers'

const small = { ...model, model: { ...model.model, id: 'small', contextWindow: 1_000 } }

function history() {
  const transcript = makeTranscript()
  for (let i = 0; i < 3; i++) {
    transcript.pushUserTurn(`turn-${i}`, model, text(`question ${i} ${'x'.repeat(2_000)}`))
    transcript.applyProviderEvent(model, events.text('answer'))
    transcript.applyProviderEvent(model, events.response())
  }
  return transcript
}

test('an unavailable previous model is never invoked to switch to a smaller model', async () => {
  const old = new RecordingProvider([[events.error('quota exhausted', 'rate_limit')]])
  const next = new RecordingProvider([[events.text('summary'), events.response()], [events.text('done'), events.response()]])
  const session = createSession(old, createRuntime(), history())
  session.updateModel(next, small)
  await session.send(text('continue'))
  expect(old.requests).toHaveLength(0)
  expect(next.requests.map((r) => r.modelId)).toEqual(['small', 'small'])
  expect(session.modelSelection).toEqual(small)
})

test('failed target compaction retains the target selection and untouched history for another attempt', async () => {
  const next = new RecordingProvider([
    [events.error('quota exhausted', 'rate_limit')],
    [events.text('summary'), events.response()],
    [events.text('done'), events.response()],
  ])
  const session = createSession(new RecordingProvider([]), createRuntime(), history())
  const before = structuredClone(session.transcript().blocks)
  session.updateModel(next, small)
  await expect(session.send(text('continue'))).rejects.toThrow('quota exhausted')
  expect(session.modelSelection).toEqual(small)
  expect(session.transcript().blocks.slice(0, before.length)).toEqual(before)
  expect(session.transcript().blocks.some((b) => b.type === 'compaction_boundary')).toBe(false)
  await session.send(text('try again'))
  expect(next.requests.map((r) => r.modelId)).toEqual(['small', 'small', 'small'])
})

test('immediate switch cancels a hanging summary and continues without aborting the parent', async () => {
  const started = deferred<void>()
  const never = deferred<void>()
  class Hanging implements AgentProvider {
    clone() { return new Hanging() }
    async *run(_request: InferenceRequest): AsyncIterable<ProviderEvent> {
      started.resolve()
      await never.promise
      yield events.text('obsolete summary')
      yield events.response()
    }
  }
  const session = createSession(new Hanging(), createRuntime(), history(), small)
  const pending = session.send(text('continue'))
  await started.promise
  const next = new RecordingProvider([[events.text('new summary'), events.response()], [events.text('done'), events.response()]])
  session.updateModel(next, { ...small, model: { ...small.model, id: 'replacement' } }, 'immediate')
  await pending
  never.resolve()
  expect(next.requests.map((r) => r.modelId)).toEqual(['replacement', 'replacement'])
  expect(session.transcript().blocks.filter((b) => b.type === 'compaction_boundary').map((b) => b.summary)).toEqual(['new summary'])
  expect(session.transcript().blocks.some((b) => b.type === 'abort')).toBe(false)
})

test('a model-only update preserves the pending cross-provider runtime', async () => {
  const old = new RecordingProvider([])
  const next = new RecordingProvider([[events.text('done'), events.response()]])
  const session = createSession(old)
  session.updateModel(next, small)
  session.updateModel(null, model)
  await session.send(text('hello'))
  expect(old.requests).toHaveLength(0)
  expect(next.requests).toHaveLength(1)
})

test('next_turn leaves an active summary alone and switches on the next send', async () => {
  const started = deferred<void>()
  const finish = deferred<void>()
  const old = new RecordingProvider([
    async function* () { started.resolve(); await finish.promise; yield events.text('summary'); yield events.response() },
    [events.text('old answer'), events.response()],
  ])
  const next = new RecordingProvider([[events.text('new answer'), events.response()]])
  const session = createSession(old, createRuntime(), history(), small)
  const sending = session.send(text('first'))
  await started.promise
  session.updateModel(next, model, 'next_turn')
  expect(session.modelSelection).toEqual(small)
  finish.resolve()
  await sending
  expect(next.requests).toHaveLength(0)
  await session.send(text('second'))
  expect(next.requests).toHaveLength(1)
})

test('context rejection shrinks summary windows and retains all historical facts', async () => {
  const transcript = history()
  const summaryRequests: string[] = []
  const next: AgentProvider = {
    clone() { return this },
    async *run(request) {
      const material = JSON.stringify(request.items.slice(0, -1))
      if (JSON.stringify(request.items.at(-1)).includes('Summarize the conversation')) {
        summaryRequests.push(material)
        if (material.length > 3_000) { yield events.error('too much history', 'context_length_exceeded'); return }
        const facts = [0, 1, 2].filter((i) => material.includes(`question ${i}`)).map((i) => `question ${i}`).join('; ')
        yield events.text(facts)
      } else yield events.text('done')
      yield events.response()
    },
  }
  const session = createSession(new RecordingProvider([]), createRuntime(), transcript)
  session.updateModel(next, small)
  await session.send(text('continue'))
  expect(summaryRequests.length).toBeGreaterThan(1)
  const finalRequest = session.transcript().replayableBlocks()
  const retained = JSON.stringify(finalRequest)
  for (let i = 0; i < 3; i++) expect(retained).toContain(`question ${i}`)
})

test('an impossible target window fails boundedly and retains the selected model', async () => {
  const next = new RecordingProvider([[events.text('summary'), events.response()]])
  const session = createSession(new RecordingProvider([]), createRuntime(), history())
  const impossible = { ...small, model: { ...small.model, contextWindow: 1 } }
  session.updateModel(next, impossible)
  await expect(session.send(text('continue'))).rejects.toThrow('no reducible history')
  expect(session.modelSelection).toEqual(impossible)
  expect(next.requests.length).toBeLessThanOrEqual(8)
})

import { afterEach, expect, test } from 'bun:test'
import { deferred } from '@demicodes/utils'
import type { AgentProvider, InferenceItem, InferenceRequest, ProviderEvent } from '@demicodes/provider'
import {
  AgentSession,
  TranscriptLog,
  applyTranscriptPatches,
  completionMessageId,
  type AgentHarnessRuntime,
  type AgentSessionPersistUpdate,
  type AgentSessionOptions,
  type EditRequest,
  type SessionEvent,
} from '../index'
import { MemorySessionStore, commandStateFor, makeTranscript, model, text } from './helpers'
import { ChildSupervisor, type ChildSupervisorOptions } from '../subagent/supervisor'
import type { AgentNodeRecord } from '../types'

const usage = { inputTokens: 10, outputTokens: 2, cacheReadTokens: 0, cacheWriteTokens: 0 }
type State = { count: number }
const cleanups: Array<() => void | Promise<void>> = []

afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) {
    await cleanup()
  }
})

class ContextProvider implements AgentProvider {
  readonly requests: InferenceRequest[] = []
  disposed = false
  failDispose = false
  failClone = false

  constructor(
    readonly family: ContextProvider[] = [],
    readonly script?: (request: InferenceRequest) => AsyncIterable<ProviderEvent>,
  ) {
    family.push(this)
  }

  clone(): AgentProvider {
    if (this.failClone) {
      throw new Error('clone failed')
    }
    return new ContextProvider(this.family, this.script)
  }

  async *run(request: InferenceRequest): AsyncIterable<ProviderEvent> {
    if (this.disposed) {
      throw new Error('A disposed provider received a request')
    }
    this.requests.push({ ...request, items: structuredClone(request.items) })
    if (this.script) {
      yield* this.script(request)
    } else {
      yield { type: 'text_delta', text: 'new-answer' }
      yield { type: 'response', usage }
    }
  }

  dispose(): void {
    this.disposed = true
    if (this.failDispose) {
      throw new Error('dispose failed')
    }
  }
}

function history(): TranscriptLog {
  const transcript = makeTranscript()
  for (const value of ['A', 'B', 'C']) {
    transcript.pushUserTurn(`turn-${value}`, model, text(value))
    transcript.applyProviderEvent(model, { type: 'text_delta', text: `answer-${value}` })
    transcript.applyProviderEvent(model, { type: 'response', usage })
  }
  return transcript
}

async function fixture(options: {
  transcript?: TranscriptLog
  store?: MemorySessionStore<State>
  provider?: ContextProvider
  runtime?: Partial<AgentHarnessRuntime<State>>
  sessionOptions?: AgentSessionOptions<State>
} = {}) {
  const transcript = options.transcript ?? history()
  const store = options.store ?? new MemorySessionStore<State>()
  await store.save({
    changedBlocks: transcript.blocks.map((block, index) => ({ index, block })),
    blockCount: transcript.blocks.length,
    commandState: commandStateFor(transcript.blocks),
    state: { count: 99 },
    phase: 'idle',
    queue: [],
    model,
    cwd: '/workspace',
    harnessName: 'editing-test',
  })
  const provider = options.provider ?? new ContextProvider()
  const runtime: AgentHarnessRuntime<State> = {
    harnessName: 'editing-test',
    initialState: () => ({ count: 0 }),
    restoreState: (ctx) => {
      const snapshot = ctx.transcript.latestExtensionStateSnapshot('counter')
      return snapshot ? structuredClone(snapshot.state) as State : { count: 0 }
    },
    systemPrompt: ({ state }) => `count=${state.count}`,
    tools: () => [],
    ...options.runtime,
  }
  const checkpoint = (await store.load())!
  const session = AgentSession.fromCheckpoint({ provider, runtime, checkpoint }, {
    agentSessionId: 'editing-session',
    store,
    retry: { maxAttempts: 1 },
    compaction: { preflightThresholdRatio: Number.POSITIVE_INFINITY },
    ...options.sessionOptions,
  })
  cleanups.push(() => session.dispose())
  const before = session.transcript().toJSON().blocks
  const frames: SessionEvent[] = []
  let clientBlocks = structuredClone(before)
  session.subscribe((event) => {
    frames.push(event)
    if (event.type === 'transcript_changed') {
      clientBlocks = applyTranscriptPatches(clientBlocks, event.patches)
    }
  })
  const request = (value = 'B', operationId = crypto.randomUUID()): EditRequest => ({
    operationId,
    targetBlockId: before.find((block) => block.type === 'user' && block.turnId === `turn-${value}`)!.id,
    version: session.transcript().version(),
    content: text(`${value}-edited`),
  })
  return { session, store, provider, before, frames, runtime, request, client: () => clientBlocks }
}

for (const [target, prefixTurns] of [['A', 0], ['B', 1], ['C', 2]] as const) {
  test(`editing ${target} preserves only its prefix in memory, patches, storage and provider input`, async () => {
    const f = await fixture()
    const request = f.request(target)
    const receipt = await f.session.editAndSend(request)
    await f.session.waitUntilDone()
    const blocks = f.session.transcript().blocks
    expect(blocks.slice(0, prefixTurns * 3)).toEqual(f.before.slice(0, prefixTurns * 3))
    expect(blocks[prefixTurns * 3]).toMatchObject({
      type: 'user', turnId: receipt.turnId, content: text(`${target}-edited`),
    })
    expect(blocks[prefixTurns * 3]!.id).not.toBe(request.targetBlockId)
    expect(receipt.turnId).not.toBe(`turn-${target}`)
    expect(f.client()).toEqual(blocks)
    expect((await f.store.load())!.transcript.blocks).toEqual(blocks)
    const expected = ['A', 'B'].slice(0, prefixTurns).flatMap<InferenceItem>((value) => [
      { type: 'user_message', content: text(value) },
      { type: 'assistant_text', modelId: model.model.id, text: `answer-${value}` },
    ])
    expect(f.provider.family[1]!.requests[0]!.items).toEqual([
      ...expected,
      { type: 'user_message', content: text(`${target}-edited`) },
    ])
    expect(f.provider.requests).toHaveLength(0)
    expect(f.provider.disposed).toBe(true)
  })
}

test('complete content is detached and preserves multiple texts, references and attachment bytes', async () => {
  const entered = deferred<void>()
  const release = deferred<void>()
  const f = await fixture({ runtime: {
    restoreState: async () => {
      entered.resolve()
      await release.promise
      return { count: 0 }
    },
  } })
  cleanups.push(() => release.resolve())
  const request = f.request()
  request.content = [
    ...text('first'),
    { type: 'document', source: { fileName: 'same.pdf', mediaType: 'application/pdf', data: new Uint8Array([1, 2]) } },
    ...text('second'),
    { type: 'reference', reference: 'file:///kept.txt' },
    { type: 'document', source: { fileName: 'same.pdf', mediaType: 'application/pdf', data: new Uint8Array([3, 4]) } },
  ]
  const expected = structuredClone(request.content)
  const pending = f.session.editAndSend(request)
  await entered.promise
  request.content.splice(0)
  release.resolve()
  await pending
  await f.session.waitUntilDone()
  expect(f.provider.family[1]!.requests[0]!.items.at(-1)).toEqual({
    type: 'user_message', content: expected,
  })
})

for (const stage of ['restoreState', 'preamble', 'resolveReferences'] as const) {
  test(`${stage} failure keeps accepted history and releases admission`, async () => {
    const f = await fixture({ runtime: { [stage]: () => { throw new Error('preparation failed') } } })
    await expect(f.session.editAndSend(f.request())).rejects.toThrow('preparation failed')
    await f.session.waitUntilDone()
    expect(f.session.transcript().blocks).toEqual(f.before)
    expect(f.client()).toEqual(f.before)
    expect((await f.store.load())!.transcript.blocks).toEqual(f.before)
    expect(f.provider.family).toHaveLength(1)
    expect(f.frames.some((event) => event.type === 'error')).toBe(false)
    f.runtime[stage] = undefined
    f.runtime.restoreState = () => ({ count: 0 })
    await f.session.editAndSend(f.request())
    await f.session.waitUntilDone()
  })
}

test('a harness must explicitly provide state reconstruction', async () => {
  const f = await fixture({ runtime: { restoreState: undefined } })
  await expect(f.session.editAndSend(f.request())).rejects.toThrow('does not support')
  await f.session.waitUntilDone()
  expect(f.session.transcript().blocks).toEqual(f.before)
})

test('state reconstruction preserves the shared root and discards suffix snapshots', async () => {
  const transcript = history()
  transcript.blocks.splice(3, 0, {
    type: 'extension_state_snapshot', id: 'prefix-state', createdAt: '2026-09-10T00:00:00Z',
    extensionName: 'counter', state: { count: 7 },
  })
  transcript.appendExtensionStateSnapshot('counter', { count: 99 })
  const f = await fixture({ transcript })
  const shared = f.session.state()
  await f.session.editAndSend(f.request())
  await f.session.waitUntilDone()
  expect(f.session.state()).toBe(shared)
  expect(shared.count).toBe(7)
  expect(f.provider.family[1]!.requests[0]!.systemPrompt).toBe('count=7')
  expect(f.session.transcript().blocks.some((block) =>
    block.type === 'extension_state_snapshot' && (block.state as State).count === 99,
  )).toBe(false)
})

class BarrierStore extends MemorySessionStore<State> {
  readonly entered = deferred<void>()
  readonly release = deferred<void>()
  fail = false
  override async save(update: AgentSessionPersistUpdate<State>): Promise<void> {
    if (update.edits?.length && !this.snapshots.at(-1)?.edits?.length) {
      this.entered.resolve()
      await this.release.promise
      if (this.fail) {
        throw new Error('save failed')
      }
    }
    super.save(update)
  }
}

for (const fail of [false, true]) {
  test(`save barrier exposes no candidate history; save ${fail ? 'failure rolls back' : 'success admits'}`, async () => {
    const store = new BarrierStore()
    store.fail = fail
    const f = await fixture({ store })
    cleanups.push(() => store.release.resolve())
    const pending = f.session.editAndSend(f.request())
    const result = pending.catch((error: unknown) => error)
    await store.entered.promise
    expect(f.session.transcript().blocks).toEqual(f.before)
    expect(f.client()).toEqual(f.before)
    expect((await store.load())!.transcript.blocks).toEqual(f.before)
    expect(f.provider.family.every((provider) => provider.requests.length === 0)).toBe(true)
    store.release.resolve()
    const outcome = await result
    await f.session.waitUntilDone()
    if (fail) {
      expect(outcome).toBeInstanceOf(Error)
      expect(f.session.transcript().blocks).toEqual(f.before)
      expect(f.client()).toEqual(f.before)
      expect((await store.load())!.transcript.blocks).toEqual(f.before)
      expect(f.provider.disposed).toBe(false)
      expect(f.provider.family[1]!.disposed).toBe(true)
    } else {
      expect(outcome).toMatchObject({ operationId: expect.any(String) })
      expect(f.client()).toEqual((await store.load())!.transcript.blocks)
    }
  })
}

test('abort during preparation releases admission without adding an abort to accepted history', async () => {
  const entered = deferred<void>()
  const release = deferred<void>()
  const f = await fixture({ runtime: { preamble: async () => {
    entered.resolve()
    await release.promise
    return 'late preamble'
  } } })
  cleanups.push(() => release.resolve())
  const pending = f.session.editAndSend(f.request()).catch((error: unknown) => error)
  await entered.promise
  await f.session.abort()
  expect(await pending).toBeInstanceOf(Error)
  await f.session.waitUntilDone()
  expect(f.session.transcript().blocks).toEqual(f.before)
  release.resolve()
  await Promise.resolve()
  expect(f.session.transcript().blocks).toEqual(f.before)
})

for (const fail of [false, true]) {
  test(`abort during commit follows the durable ${fail ? 'failure' : 'success'}`, async () => {
    const store = new BarrierStore()
    store.fail = fail
    const f = await fixture({ store })
    cleanups.push(() => store.release.resolve())
    const pending = f.session.editAndSend(f.request()).catch((error: unknown) => error)
    await store.entered.promise
    await f.session.abort()
    store.release.resolve()
    const result = await pending
    await f.session.waitUntilDone()
    expect(f.provider.family.every((provider) => !provider.requests.length)).toBe(true)
    if (fail) {
      expect(result).toBeInstanceOf(Error)
      expect(f.session.transcript().blocks).toEqual(f.before)
    } else {
      expect(result).toMatchObject({ turnId: expect.any(String) })
      expect(f.session.transcript().blocks.at(-1)?.type).toBe('abort')
      expect((await store.load())!.edits).toHaveLength(1)
    }
  })
}

test('dispose waits for an in-flight edit save before releasing the accepted runtime', async () => {
  const store = new BarrierStore()
  const f = await fixture({ store })
  cleanups.push(() => store.release.resolve())
  const pending = f.session.editAndSend(f.request())
  await store.entered.promise
  let disposed = false
  const disposal = f.session.dispose().then(() => {
    disposed = true
  })
  await Promise.resolve()
  expect(disposed).toBe(false)
  expect(f.provider.family.every((provider) => !provider.disposed)).toBe(true)
  store.release.resolve()
  await pending
  await disposal
  expect(disposed).toBe(true)
  expect(f.provider.family.every((provider) => provider.disposed)).toBe(true)
  expect(f.provider.family.every((provider) => provider.requests.length === 0)).toBe(true)
  expect((await store.load())!.transcript.blocks).toEqual(f.session.transcript().blocks)
  expect((await store.load())!.edits).toHaveLength(1)
})

test('accepted IDs remain idempotent across restart and a later edit removing the replacement', async () => {
  const f = await fixture()
  const request = f.request()
  const receipt = await f.session.editAndSend(request)
  await f.session.waitUntilDone()
  await expect(f.session.editAndSend(request)).resolves.toEqual(receipt)
  await f.session.editAndSend(f.request('A'))
  await f.session.waitUntilDone()
  const after = f.session.transcript().toJSON()
  const checkpoint = (await f.store.load())!
  const provider = new ContextProvider()
  const restored = AgentSession.fromCheckpoint({ provider, runtime: f.runtime, checkpoint }, { store: f.store })
  cleanups.push(() => restored.dispose())
  await expect(restored.editAndSend(request)).resolves.toEqual(receipt)
  expect(restored.transcript().toJSON()).toEqual(after)
  expect(provider.family).toHaveLength(1)
  await expect(restored.editAndSend({ ...request, content: text('different') })).rejects.toThrow('different request')
  await expect(restored.editAndSend({ ...request, operationId: 'fresh-id' })).rejects.toThrow('conversation changed')
})

test('repeated in-flight operation IDs share one acceptance and reject conflicting content', async () => {
  const store = new BarrierStore()
  const f = await fixture({ store })
  cleanups.push(() => store.release.resolve())
  const request = f.request()
  const first = f.session.editAndSend(request)
  await store.entered.promise
  const second = f.session.editAndSend(request)
  await expect(f.session.editAndSend({ ...request, content: text('other') })).rejects.toThrow('different request')
  store.release.resolve()
  expect(await second).toEqual(await first)
  await f.session.waitUntilDone()
  expect((await store.load())!.edits).toHaveLength(1)
  expect(f.provider.family).toHaveLength(2)
})

test('generation and discarded-provider disposal failures preserve the accepted replacement', async () => {
  for (const cause of ['generation', 'disposal']) {
    const provider = new ContextProvider([], async function* () {
      throw new Error('generation failed')
    })
    provider.failDispose = cause === 'disposal'
    const f = await fixture({ provider })
    const receipt = await f.session.editAndSend(f.request())
    await f.session.waitUntilDone()
    expect(f.session.transcript().blocks.some((block) =>
      block.type === 'user' && block.turnId === receipt.turnId,
    )).toBe(true)
    expect((await f.store.load())!.edits).toHaveLength(1)
    expect(f.frames.some((event) => event.type === 'error'
      && event.error.message === `${cause === 'disposal' ? 'dispose' : 'generation'} failed`)).toBe(true)
  }
})

test('edit admission excludes sends, steers, model changes and wakeups during preparation', async () => {
  const store = new BarrierStore()
  const f = await fixture({ store })
  cleanups.push(() => store.release.resolve())
  const pending = f.session.editAndSend(f.request())
  await store.entered.promise
  await expect(f.session.send(text('racing-send'))).rejects.toThrow()
  await expect(f.session.steer(text('racing-steer'))).rejects.toThrow()
  expect(() => f.session.updateModel(null, model)).toThrow()
  expect(() => f.session.scheduleYieldWakeup(100)).toThrow()
  expect(() => f.session.reserveMutation()).toThrow()
  store.release.resolve()
  await pending
  await f.session.waitUntilDone()
  expect(f.session.queuedMessages()).toHaveLength(0)
})

test('a pending wakeup prevents editing without consuming the wakeup', async () => {
  const f = await fixture()
  f.session.scheduleYieldWakeup(60_000)
  await expect(f.session.editAndSend(f.request())).rejects.toThrow('pending work')
  expect(f.session.hasPendingYields()).toBe(true)
  expect(f.session.transcript().blocks).toEqual(f.before)
})

for (const kind of ['hidden', 'completion', 'assistant', 'missing', 'steer'] as const) {
  test(`${kind} blocks are not editable targets`, async () => {
    const transcript = history()
    if (kind === 'hidden') {
      transcript.pushUserTurn('hidden', model, text('wakeup'), null, true)
    } else if (kind === 'completion') {
      transcript.pushUserTurn(completionMessageId('child'), model, text('child finished'))
    } else if (kind === 'steer') {
      transcript.pushSteer('turn-C', model, text('steer'))
    }
    const f = await fixture({ transcript })
    const targetBlockId = kind === 'missing' ? 'missing' : kind === 'assistant'
      ? f.before.find((block) => block.type === 'text')!.id
      : f.before.at(-1)!.id
    await expect(f.session.editAndSend({ ...f.request(), targetBlockId })).rejects.toThrow('user message')
    await f.session.waitUntilDone()
    expect(f.session.transcript().blocks).toEqual(f.before)
  })
}

test('a stale editor is rejected after another edit and after a runtime restart', async () => {
  const f = await fixture()
  const stale = f.request('C')
  await f.session.editAndSend(f.request())
  await f.session.waitUntilDone()
  await expect(f.session.editAndSend(stale)).rejects.toThrow('conversation changed')
  const latest = f.request('A')
  const restored = AgentSession.fromCheckpoint({
    provider: new ContextProvider(), runtime: f.runtime, checkpoint: (await f.store.load())!,
  })
  cleanups.push(() => restored.dispose())
  await expect(restored.editAndSend(latest)).rejects.toThrow('conversation changed')
})

test('prefix mutations in preparation are rejected', async () => {
  const f = await fixture({ runtime: { preamble: ({ transcript }) => {
    transcript.blocks.splice(0, 1)
    return null
  } } })
  await expect(f.session.editAndSend(f.request())).rejects.toThrow('retained transcript')
  await f.session.waitUntilDone()
  expect(f.session.transcript().blocks).toEqual(f.before)
})

test('references are resolved again while the editor content stays unexpanded', async () => {
  let file = 'old-file'
  const f = await fixture({ runtime: {
    resolveReferences: (_ctx, content) => content.flatMap((part) =>
      part.type === 'reference' ? text(file) : [part],
    ),
  } })
  const content = [{ type: 'reference' as const, reference: 'file:///notes.txt' }]
  await f.session.send(content, { id: 'file-turn' })
  const original = f.session.transcript().blocks.find((block) =>
    block.type === 'user' && block.turnId === 'file-turn',
  )!
  expect(original).toMatchObject({ content, resolvedContent: text('old-file') })
  file = 'new-file'
  await f.session.editAndSend({
    ...f.request(),
    targetBlockId: original.id,
    version: f.session.transcript().version(),
    content,
  })
  await f.session.waitUntilDone()
  expect(f.provider.family[1]!.requests[0]!.items.at(-1)).toEqual({
    type: 'user_message', content: text('new-file'),
  })
  expect((await f.store.load())!.transcript.blocks.slice().reverse().find((block) =>
    block.type === 'user',
  )).toMatchObject({ content, resolvedContent: text('new-file') })
})

for (const target of ['A', 'B', 'C'] as const) {
  test(`edit ${target} respects multiple compaction boundaries and a removed marker`, async () => {
    const transcript = history()
    const first = transcript.insertCompactionBoundary(3, model, 'summary-A', 3)
    transcript.appendCompactionMarker(model, first.id, 500)
    const second = transcript.insertCompactionBoundary(7, model, 'summary-A-B', 4)
    transcript.appendCompactionMarker(model, second.id, 700)
    const f = await fixture({ transcript })
    const targetIndex = f.before.findIndex((block) =>
      block.type === 'user' && block.turnId === `turn-${target}`,
    )
    await f.session.editAndSend(f.request(target))
    await f.session.waitUntilDone()
    expect(f.session.transcript().blocks.slice(0, targetIndex)).toEqual(f.before.slice(0, targetIndex))
    const expected: InferenceItem[] = target === 'A' ? [] : [{
      type: 'user_message',
      content: text(`Previous conversation summary:\n${target === 'B' ? 'summary-A' : 'summary-A-B'}`),
    }]
    expect(f.provider.family[1]!.requests[0]!.items).toEqual([
      ...expected,
      { type: 'user_message', content: text(`${target}-edited`) },
    ])
    expect(f.client()).toEqual((await f.store.load())!.transcript.blocks)
  })
}

test('a removed compaction marker invalidates old measured usage in the retained prefix', () => {
  const transcript = history()
  const boundary = transcript.insertCompactionBoundary(3, model, 'summary-A', 2)
  transcript.appendCompactionMarker(model, boundary.id, 200)
  const target = transcript.blocks.find((block) => block.type === 'user' && block.turnId === 'turn-C')!
  const candidate = transcript.beforeUserMessage(target.id)
  const response = candidate.blocks.slice().reverse().find((block) => block.type === 'response')!
  if (response.type !== 'response') throw new Error('missing response')
  response.usage.inputTokens = 90_000
  expect(candidate.estimateContextTokens()).toBeLessThan(100)
})

test('fixed-seed histories cut at the selected user and keep patches and reload identical', async () => {
  let seed = 0x20260911
  const next = () => {
    seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0
    return seed
  }
  for (let sample = 0; sample < 16; sample += 1) {
    const transcript = makeTranscript()
    const count = 2 + next() % 7
    const positions: number[] = []
    for (let turn = 0; turn < count; turn += 1) {
      positions.push(transcript.blocks.length)
      transcript.pushUserTurn(`sample-${sample}-${turn}`, model, text(`question-${turn}`))
      transcript.pushSteer(`sample-${sample}-${turn}`, model, text(`steer-${turn}`))
      transcript.applyProviderEvent(model, { type: 'text_delta', text: `answer-${turn}` })
      transcript.applyProviderEvent(model, { type: 'response', usage })
      transcript.appendExtensionStateSnapshot('counter', { count: turn })
    }
    const index = positions[next() % count]!
    const f = await fixture({ transcript })
    const content = text(`replacement-${sample}`)
    const receipt = await f.session.editAndSend({
      operationId: crypto.randomUUID(), targetBlockId: f.before[index]!.id,
      version: f.session.transcript().version(), content,
    })
    await f.session.waitUntilDone()
    expect(f.session.transcript().blocks.slice(0, index)).toEqual(f.before.slice(0, index))
    expect(f.session.transcript().blocks.slice(index).map((block) => block.type)).toEqual(['user', 'text', 'response'])
    expect(f.session.transcript().blocks[index]).toMatchObject({ content, turnId: receipt.turnId })
    expect(f.client()).toEqual((await f.store.load())!.transcript.blocks)
  }
})

test('provider creation failure releases admission and preserves the consumed runtime', async () => {
  const provider = new ContextProvider()
  provider.failClone = true
  const f = await fixture({ provider })
  await expect(f.session.editAndSend(f.request())).rejects.toThrow('clone failed')
  await f.session.waitUntilDone()
  expect(f.session.transcript().blocks).toEqual(f.before)
  expect(provider.disposed).toBe(false)
  provider.failClone = false
  await f.session.editAndSend(f.request())
  await f.session.waitUntilDone()
})

test('preflight compaction after an edit summarizes only retained history', async () => {
  const provider = new ContextProvider([], async function* (request) {
    const last = request.items.at(-1)
    const summary = last?.type === 'user_message'
      && last.content.some((part) => part.type === 'text' && part.text.includes('summary'))
    yield { type: 'text_delta', text: summary ? 'summary-kept-A' : 'edited-answer' }
    yield { type: 'response', usage }
  })
  const f = await fixture({ provider, sessionOptions: { compaction: {
    preflightThresholdRatio: 0.8, preflightThresholdTokens: 1, keepRecentTokens: 1,
  } } })
  await f.session.editAndSend(f.request())
  await f.session.waitUntilDone()
  const requests = provider.family.flatMap((runtime) => runtime.requests)
  expect(requests.length).toBeGreaterThanOrEqual(2)
  for (const request of requests) {
    const payload = JSON.stringify(request.items)
    expect(payload).not.toContain('answer-B')
    expect(payload).not.toContain('answer-C')
  }
  const summaryRequest = provider.family[2]!.requests[0]!
  expect(summaryRequest.items.slice(0, -1)).toEqual([
    { type: 'user_message', content: text('A') },
    { type: 'assistant_text', modelId: model.model.id, text: 'answer-A' },
  ])
})

test('an earlier checkpoint in flight settles before the edit save and cannot restore removed rows', async () => {
  class PriorStore extends MemorySessionStore<State> {
    armed = false
    readonly entered = deferred<void>()
    readonly release = deferred<void>()
    override async save(update: AgentSessionPersistUpdate<State>): Promise<void> {
      if (this.armed) {
        this.armed = false
        this.entered.resolve()
        await this.release.promise
      }
      super.save(update)
    }
  }
  const store = new PriorStore()
  const transcript = history()
  transcript.applyProviderEvent(model, { type: 'tool_call_requested', toolUseId: 'lost', toolName: 'lost', input: {} })
  const f = await fixture({ transcript, store, sessionOptions: { persistIntervalMs: 0 } })
  cleanups.push(() => store.release.resolve())
  store.armed = true
  // Publish the restored tool repair through the normal checkpoint scheduler
  // without starting an action, then hold that already captured checkpoint.
  await (f.session as unknown as { commitTranscript(): Promise<void> }).commitTranscript()
  await store.entered.promise
  const pending = f.session.editAndSend(f.request())
  await Promise.resolve()
  expect(f.provider.family).toHaveLength(1)
  expect(f.session.transcript().blocks).toEqual(f.before)
  store.release.resolve()
  await pending
  await f.session.waitUntilDone()
  const loaded = (await store.load())!
  expect(loaded.transcript.blocks).toEqual(f.client())
  expect(loaded.edits).toHaveLength(1)
  expect(loaded.transcript.blocks.map((block) => block.type)).toEqual(['user', 'text', 'response', 'user', 'text', 'response'])
})

test('a failed continuation retains a completed tool and receipt; retrying submission cannot rerun the tool', async () => {
  let calls = 0
  let invocations = 0
  const provider = new ContextProvider([], async function* () {
    calls += 1
    if (calls === 1) {
      yield { type: 'tool_call_requested', toolUseId: 'effect', toolName: 'effect', input: {} }
      yield { type: 'response', usage }
    } else if (calls === 2) {
      throw new Error('continuation failed')
    } else {
      yield { type: 'text_delta', text: 'recovered' }
      yield { type: 'response', usage }
    }
  })
  const f = await fixture({ provider, runtime: {
    tools: () => [{
      name: 'effect', description: 'count one durable effect', inputSchema: { type: 'object' },
      invoke: () => {
        invocations += 1
        return { output: [{ type: 'text', text: 'permanent-result' }] }
      },
    }],
  } })
  const request = f.request()
  const receipt = await f.session.editAndSend(request)
  await f.session.waitUntilDone()
  expect(invocations).toBe(1)
  await expect(f.session.editAndSend(request)).resolves.toEqual(receipt)
  expect(calls).toBe(2)
  await f.session.resume()
  expect(invocations).toBe(1)
  expect(f.provider.family[1]!.requests[2]!.items).toContainEqual({
    type: 'tool_result', toolUseId: 'effect', output: [{ type: 'text', text: 'permanent-result' }], isError: false,
  })
})

function supervisorFor(children: () => Promise<AgentNodeRecord[]>): ChildSupervisor<State> {
  // These tests exercise admission and empty-child restoration only. No child
  // assembly, command environment or provider dependency is used on those paths.
  return new ChildSupervisor({
    ownerId: 'root', tree: { store: { children }, profiles: [] },
  } as unknown as ChildSupervisorOptions<State>)
}

for (const pending of ['live', 'undelivered', 'delivered'] as const) {
  test(`${pending} child record controls edit admission without changing the record`, async () => {
    const child: AgentNodeRecord = {
      id: 'child', parentId: 'root', description: '', profileName: null, metadata: null,
      spawnedAt: 1, canSpawnSubagents: false, closedPhase: pending === 'live' ? null : 'completed',
      closedAt: pending === 'live' ? null : 2, result: 'permanent-child-result', failure: null,
      delivered: pending === 'delivered',
    }
    const before = structuredClone(child)
    const supervisor = supervisorFor(async () => [structuredClone(child)])
    const f = await fixture({ runtime: { reserveEdit: () => supervisor.reserveEdit() } })
    if (pending === 'delivered') {
      await f.session.editAndSend(f.request())
    } else {
      await expect(f.session.editAndSend(f.request())).rejects.toThrow('children or completion')
      expect(f.session.transcript().blocks).toEqual(f.before)
    }
    await f.session.waitUntilDone()
    expect(child).toEqual(before)
  })
}

for (const first of ['edit', 'restore'] as const) {
  test(`${first} admission excludes the competing child restoration until release`, async () => {
    const entered = deferred<void>()
    const release = deferred<void>()
    const supervisor = supervisorFor(async () => {
      entered.resolve()
      await release.promise
      return []
    })
    const f = await fixture()
    supervisor.attachParent(f.session)
    const admitted = first === 'edit' ? supervisor.reserveEdit() : supervisor.restore()
    try {
      await entered.promise
      if (first === 'edit') {
        await expect(supervisor.restore()).rejects.toThrow('edit is being prepared')
      } else {
        await expect(supervisor.reserveEdit()).rejects.toThrow('lifecycle operation')
      }
    } finally {
      release.resolve()
      const reservation = await admitted
      reservation?.release()
    }
    const next = await supervisor.reserveEdit()
    next.release()
  })
}

test('active generation and queued sends prevent editing without losing work', async () => {
  const entered = deferred<void>()
  const release = deferred<void>()
  const provider = new ContextProvider([], async function* () {
    entered.resolve()
    await release.promise
    yield { type: 'text_delta', text: 'finished' }
    yield { type: 'response', usage }
  })
  const f = await fixture({ provider })
  cleanups.push(() => release.resolve())
  const active = f.session.send(text('D'))
  await entered.promise
  const queued = f.session.send(text('E'))
  await expect(f.session.editAndSend(f.request())).rejects.toThrow('pending work')
  expect(f.session.queuedMessages()).toHaveLength(1)
  release.resolve()
  await Promise.all([active, queued])
})

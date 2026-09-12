import { afterEach, expect, test } from 'bun:test'
import type { AgentMessage } from '@demicodes/core'
import { type AgentProvider, type InferenceSteer } from '@demicodes/provider'
import { StubProvider, createProviderRun, events } from '@demicodes/provider/testing'
import { deferred } from '@demicodes/utils'
import { AgentSession } from '../session/session'
import { agentMessageContent } from '../transcript/agent-message'
import { createRuntime, createSession, MemorySessionStore, makeTranscript, RecordingProvider, model, text, type TestState } from './helpers'

const sessions: AgentSession<TestState>[] = []
afterEach(async () => {
  for (const session of sessions.splice(0)) await session.dispose()
})

function message(id: string, completion = false): AgentMessage {
  return {
    id: completion ? 'subagent:child:1' : id,
    sender: { id: 'child', description: 'UI implementation', round: 1 },
    recipientId: 'parent',
    timestamp: '2026-09-12T10:00:00.000Z',
    content: `Result ${id}`,
    event: completion ? { type: 'completion', outcome: 'completed' } : { type: 'message' },
  }
}

function setup(provider: AgentProvider, runtime = createRuntime(), store = new MemorySessionStore<TestState>()) {
  const session = createSession(provider, runtime, undefined, model, { store, agentSessionId: 'parent' })
  sessions.push(session)
  return { session, store }
}

test('busy tool receives an ordered durable batch in its continuation with no human queue', async () => {
  const started = deferred<void>()
  const release = deferred<void>()
  let continued = false
  const first = message('update')
  const second = message('completion', true)
  const provider = new StubProvider([
    [events.toolCall('tool', 'hold', {}), events.response()],
    request => {
      const inputs = request.items.filter(item => item.type === 'user_steer')
      expect(inputs.map(item => item.content)).toEqual([agentMessageContent(first), agentMessageContent(second)])
      continued = true
      return [events.text('Finished with both results'), events.response()]
    },
  ])
  const { session, store } = setup(provider, createRuntime({
    tools: () => [{
      name: 'hold', description: 'A gated test tool', inputSchema: {},
      invoke: async () => {
        started.resolve()
        await release.promise
        return { output: [{ type: 'text' as const, text: 'tool completed without interruption' }] }
      },
    }],
  }))
  const turn = session.send(text('Implement it'))
  await started.promise
  await session.acceptAgentMessage(first)
  await session.acceptAgentMessage(second)
  expect(continued).toBe(false)
  expect(session.queuedMessages()).toEqual([])
  expect(session.pendingSteers()).toEqual([])
  expect(store.snapshots.at(-1)?.pendingInternalSteers?.map(input => input.agentMessage.id))
    .toEqual(['update', 'subagent:child:1'])
  expect(session.cancelPendingSteer(first.id)).toBe(false)
  release.resolve()
  await turn
  const blocks = session.transcript().blocks
  expect(blocks.filter(block => block.type === 'user')).toHaveLength(1)
  expect(blocks.filter(block => block.type === 'agent_message').map(block => block.id)).toEqual(['update', 'subagent:child:1'])
  expect(store.snapshots.at(-1)?.pendingInternalSteers).toEqual([])
  expect(continued).toBe(true)
})

test('an idle batch opens one internal continuation and duplicate admission does not wake again', async () => {
  let requests = 0
  const { session, store } = setup(new StubProvider([
    request => {
      requests += 1
      expect(request.items.filter(item => item.type === 'user_message')).toHaveLength(0)
      expect(request.items.filter(item => item.type === 'user_steer')).toHaveLength(2)
      return [events.text('Combined result'), events.response()]
    },
  ]))
  await Promise.all([session.acceptAgentMessage(message('one')), session.acceptAgentMessage(message('two'))])
  await session.waitUntilDone()
  await session.acceptAgentMessage(message('one'))
  expect(requests).toBe(1)
  expect(store.snapshots.at(-1)?.transcript.blocks.filter(block => block.type === 'agent_message')).toHaveLength(2)
  expect(session.queuedMessages()).toEqual([])
})

test('user abort retains unread input through restore until explicit continuation', async () => {
  const started = deferred<void>()
  const release = deferred<void>()
  const { session, store } = setup(new StubProvider([
    () => (async function* () {
      started.resolve()
      await release.promise
      yield events.response()
    })(),
  ]))
  const turn = session.send(text('Start work'))
  await started.promise
  await session.acceptAgentMessage(message('unread'))
  await session.abort()
  release.resolve()
  await turn
  await session.acceptAgentMessage(message('late'))
  expect(session.transcript().blocks.filter(block => block.type === 'agent_message')).toHaveLength(0)
  const checkpoint = await store.load()
  expect(checkpoint?.pendingInternalSteers).toHaveLength(2)
  let requests = 0
  const restored = AgentSession.fromCheckpoint({
    checkpoint: checkpoint!,
    runtime: createRuntime(),
    provider: new StubProvider([request => {
      requests += 1
      expect(JSON.stringify(request.items)).toContain('Result unread')
      expect(JSON.stringify(request.items)).toContain('Result late')
      return [events.text('Continued by user'), events.response()]
    }]),
  }, { agentSessionId: 'parent', store: new MemorySessionStore<TestState>() })
  sessions.push(restored)
  restored.wakePendingAgentMessages()
  expect(requests).toBe(0)
  await restored.resume()
  expect(requests).toBe(1)
  expect(restored.transcript().blocks.filter(block => block.type === 'agent_message')).toHaveLength(2)
})

test('live provider steering and committed replay retain the same source envelope', async () => {
  const started = deferred<void>()
  const release = deferred<void>()
  const steers: InferenceSteer[] = []
  const provider: AgentProvider = {
    clone: () => provider,
    run: () => createProviderRun((async function* () {
      started.resolve()
      await release.promise
      yield events.text('Done')
      yield events.response()
    })(), { steer: input => { steers.push(input) } }),
  }
  const { session, store } = setup(provider)
  const turn = session.send(text('Work'))
  await started.promise
  const input = message('live')
  await session.acceptAgentMessage(input)
  expect(steers).toHaveLength(1)
  expect(steers[0]?.content).toEqual(agentMessageContent(input))
  expect(store.snapshots.at(-1)?.transcript.blocks.find(block => block.type === 'agent_message'))
    .toMatchObject({ id: input.id, message: input })
  expect(session.transcript().collectInferenceItems().find(item => item.type === 'user_steer'))
    .toMatchObject({ content: agentMessageContent(input) })
  release.resolve()
  await turn
})

test('admission crossing a finishing turn is retained and wakes the next continuation', async () => {
  const finalizing = deferred<void>()
  const releaseSave = deferred<void>()
  let delayed = false
  class GatedStore extends MemorySessionStore<TestState> {
    override async save(update: Parameters<MemorySessionStore<TestState>['save']>[0]) {
      if (update.phase === 'idle' && !delayed) {
        delayed = true
        finalizing.resolve()
        await releaseSave.promise
      }
      super.save(update)
    }
  }
  const provider = new StubProvider([
    [events.text('First response'), events.response()],
    [events.text('Receipt consumed'), events.response()],
  ])
  const { session } = setup(provider, createRuntime(), new GatedStore())
  const turn = session.send(text('Work'))
  await finalizing.promise
  const acceptance = session.acceptAgentMessage(message('race'))
  releaseSave.resolve()
  await acceptance
  await turn
  await session.waitUntilDone()
  expect(session.transcript().blocks.filter(block => block.type === 'agent_message')).toHaveLength(1)
  expect(session.transcript().blocks.some(block => block.type === 'text' && block.text === 'Receipt consumed')).toBe(true)
})

test('admission rejects malformed envelopes and mismatched recipients', async () => {
  const { session } = setup(new StubProvider([]))
  await expect(session.acceptAgentMessage({ ...message('bad'), recipientId: 'elsewhere' })).rejects.toThrow('recipient')
  await expect(session.acceptAgentMessage({ ...message('bad'), timestamp: 'yesterday' })).rejects.toThrow()
  expect(session.hasPendingAgentMessages()).toBe(false)
})

test('retry of a failed internal continuation preserves the completed human turn and all receipts', async () => {
  const { session } = setup(new StubProvider([
    [events.text('Original task answer'), events.response()],
    () => { throw new Error('scripted provider failure') },
    request => {
      expect(JSON.stringify(request.items)).toContain('Original task answer')
      expect(request.items.filter(item => item.type === 'user_steer')).toHaveLength(1)
      return [events.text('Retried receipt'), events.response()]
    },
  ]))
  await session.send(text('Original task'))
  await session.acceptAgentMessage(message('retry'))
  await session.waitUntilDone()
  await session.retry()
  expect(session.transcript().blocks.filter(block => block.type === 'user')).toHaveLength(1)
  expect(session.transcript().blocks.filter(block => block.type === 'agent_message')).toHaveLength(1)
  expect(session.transcript().blocks.some(block => block.type === 'text' && block.text === 'Original task answer')).toBe(true)
})

test('restore of durable idle input wakes once and restore of materialized input never replays delivery', async () => {
  const input = message('restored')
  const original = setup(new StubProvider([[events.text('Original answer'), events.response()]]))
  await original.session.send(text('Task'))
  const checkpoint = (await original.store.load())!
  checkpoint.pendingInternalSteers = [{ turnId: input.id, model, agentMessage: input, metadata: null }]
  let requests = 0
  const store = new MemorySessionStore<TestState>()
  // Seed the store's unchanged rows, matching a persisted node restored by assembly.
  store.save({
    ...checkpoint,
    changedBlocks: checkpoint.transcript.blocks.map((block, index) => ({ block, index })),
    blockCount: checkpoint.transcript.blocks.length,
  })
  const restored = AgentSession.fromCheckpoint({
    checkpoint,
    runtime: createRuntime(),
    provider: new StubProvider([request => {
      requests += 1
      expect(JSON.stringify(request.items)).toContain(input.content)
      return [events.text('Recovered'), events.response()]
    }]),
  }, { agentSessionId: 'parent', store })
  sessions.push(restored)
  restored.wakePendingAgentMessages()
  await restored.waitUntilDone()
  await restored.acceptAgentMessage(input)
  const final = (await store.load())!
  const again = AgentSession.fromCheckpoint({
    checkpoint: final, runtime: createRuntime(), provider: new StubProvider([]),
  }, { agentSessionId: 'parent' })
  sessions.push(again)
  again.wakePendingAgentMessages()
  await again.acceptAgentMessage(input)
  expect(requests).toBe(1)
  expect(again.transcript().blocks.filter(block => block.type === 'agent_message')).toHaveLength(1)
  expect(again.isSettled()).toBe(true)
})

test('input admitted during standalone compaction remains outside the summary and continues afterwards', async () => {
  const started = deferred<void>()
  const release = deferred<void>()
  const transcript = makeTranscript()
  transcript.pushUserTurn('old', model, text('Old task context '.repeat(100)))
  transcript.applyProviderEvent(model, events.text('Old answer '.repeat(100)))
  transcript.applyProviderEvent(model, events.response())
  transcript.pushUserTurn('recent', model, text('Current task'))
  const provider = new RecordingProvider([
    request => (async function* () {
      expect(JSON.stringify(request.items)).not.toContain('Result during-compaction')
      started.resolve()
      await release.promise
      yield events.text('Summary of the earlier task')
      yield events.response()
    })(),
    request => {
      expect(JSON.stringify(request.items)).toContain('Result during-compaction')
      return [events.text('Continued after compaction'), events.response()]
    },
  ])
  const store = new MemorySessionStore<TestState>()
  const session = createSession(provider, createRuntime(), transcript, model, { store, agentSessionId: 'parent' })
  sessions.push(session)
  const compacting = session.compact()
  await started.promise
  await session.acceptAgentMessage(message('during-compaction'))
  expect(store.snapshots.at(-1)?.pendingInternalSteers).toHaveLength(1)
  expect(session.pendingSteers()).toEqual([])
  release.resolve()
  await compacting
  expect(provider.requests).toHaveLength(2)
  const receiptIndex = session.transcript().blocks.findIndex(block => block.type === 'agent_message')
  const boundaryIndex = session.transcript().blocks.findIndex(block => block.type === 'compaction_boundary')
  expect(receiptIndex).toBeGreaterThan(boundaryIndex)
})

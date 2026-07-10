import { expect, test } from 'bun:test'
import type { Block, ModelSelection } from '@demicodes/core'
import { StubProvider, events } from '@demicodes/provider/testing'
import { waitFor } from '@demicodes/utils'
import { AgentClient, TranscriptLog, applyTranscriptPatches, createInProcessTransportPair } from '../index'
import { AgentSession } from '../session'
import type { AgentHarnessRuntime, AgentSessionCheckpoint, AgentSessionStore } from '../types'
import type { ClientFrame } from '../frames'

const model: ModelSelection = {
  providerId: 'stub',
  model: {
    id: 'test-model',
    name: 'Test Model',
    contextWindow: 1_000_000,
    inputLimit: null,
    thinking: [],
    acceptedExtensions: [],
  },
  thinking: null,
}

function createRuntime(): AgentHarnessRuntime<Record<string, never>> {
  return {
    harnessName: 'pipeline-test',
    initialState: () => ({}),
    systemPrompt: () => 'test',
    tools: () => [],
  }
}

class CountingStore implements AgentSessionStore<Record<string, never>> {
  saves = 0
  last: AgentSessionCheckpoint<Record<string, never>> | null = null

  saveCheckpoint(snapshot: AgentSessionCheckpoint<Record<string, never>>): void {
    this.saves += 1
    this.last = snapshot
  }

  loadCheckpoint(): Promise<AgentSessionCheckpoint<Record<string, never>> | null> {
    return Promise.resolve(null)
  }
}

test('streaming delta cost does not scale with transcript length', () => {
  const measure = (historyBlocks: number, deltas: number): number => {
    const history: Block[] = []
    for (let i = 0; i < historyBlocks; i += 1) {
      history.push({
        type: 'text',
        id: `text-${i}`,
        createdAt: '2026-01-01T00:00:00.000Z',
        model,
        text: 'previous assistant output line with some content in it',
      })
    }
    const transcript = new TranscriptLog(history)
    const startedAt = performance.now()
    for (let i = 0; i < deltas; i += 1) {
      transcript.applyProviderEvent(model, { type: 'text_delta', text: `delta ${i} ` })
      transcript.takePatches()
    }
    return performance.now() - startedAt
  }

  // Warm up JIT, then compare a short history against a 4000-block history.
  measure(10, 500)
  const small = measure(10, 5_000)
  const large = measure(4_000, 5_000)
  // Journal-based patches make per-delta cost independent of history size.
  // Allow generous headroom for timer noise; the pre-refactor snapshot+diff
  // pipeline was two orders of magnitude apart on this workload.
  expect(large).toBeLessThan(Math.max(small * 25, 250))
})

test('a turn with tool calls persists at action boundaries, not per event', async () => {
  const store = new CountingStore()
  const provider = new StubProvider([
    [events.toolCall('tool-1', 'noop', {}), events.response()],
    [events.toolCall('tool-2', 'noop', {}), events.response()],
    [events.text('done'), events.response()],
  ])
  const runtime: AgentHarnessRuntime<Record<string, never>> = {
    ...createRuntime(),
    tools: () => [
      {
        name: 'noop',
        description: 'does nothing',
        inputSchema: { type: 'object' },
        invoke: () => ({ output: [{ type: 'text', text: 'ok' }] }),
      },
    ],
  }
  const session = new AgentSession(
    { provider, model, cwd: '/workspace', runtime },
    { store, persistIntervalMs: 60_000 },
  )

  await session.send([{ type: 'text', text: 'run tools' }])

  // With the persist timer effectively disabled, only the boundary flush writes.
  expect(store.saves).toBe(1)
  expect(store.last?.transcript.blocks.map((block) => block.type)).toEqual([
    'user',
    'tool_call',
    'response',
    'tool_call',
    'response',
    'text',
    'response',
  ])
})

test('client resyncs with a snapshot when the patch revision stream has a gap', async () => {
  const pair = createInProcessTransportPair()
  const client = new AgentClient(pair.client)
  const receivedFrames: ClientFrame[] = []
  pair.server.onFrame((frame) => receivedFrames.push(frame))

  const blockAt = (id: string, text: string): Block => ({
    type: 'text',
    id,
    createdAt: '2026-01-01T00:00:00.000Z',
    model,
    text,
  })

  pair.server.send({ type: 'transcript_reset', revision: 1, blocks: [blockAt('a', 'one')] })
  await waitFor(() => client.transcript().blocks.length === 1)

  // Deliver revision 3 with revision 2 missing: the client must not apply it.
  pair.server.send({
    type: 'transcript_patch',
    revision: 3,
    patches: [{ op: 'add', path: ['blocks', 1], value: blockAt('c', 'three') }],
  })
  await waitFor(() => receivedFrames.some((frame) => frame.type === 'sync_transcript'))
  expect(client.transcript().blocks.map((block) => block.id)).toEqual(['a'])

  // The server answers the resync with an authoritative snapshot.
  pair.server.send({
    type: 'transcript_reset',
    revision: 3,
    blocks: [blockAt('a', 'one'), blockAt('b', 'two'), blockAt('c', 'three')],
  })
  await waitFor(() => client.transcript().blocks.length === 3)

  // Patches resume against the synced revision.
  pair.server.send({
    type: 'transcript_patch',
    revision: 4,
    patches: [{ op: 'append_text', path: ['blocks', 2], delta: ' more' }],
  })
  await waitFor(() => {
    const block = client.transcript().blocks[2]
    return block?.type === 'text' && block.text === 'three more'
  })
})

test('append_text patches replace the target block instead of mutating it', () => {
  const original: Block = {
    type: 'text',
    id: 'a',
    createdAt: '2026-01-01T00:00:00.000Z',
    model,
    text: 'start',
  }
  const applied = applyTranscriptPatches([original], [{ op: 'append_text', path: ['blocks', 0], delta: ' end' }])
  expect(applied[0]).toMatchObject({ text: 'start end' })
  expect(original.text).toBe('start')
})

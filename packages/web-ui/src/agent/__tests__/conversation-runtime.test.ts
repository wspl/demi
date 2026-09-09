import { expect, test } from 'bun:test'
import {
  AgentClient,
  type ClientFrame,
  type ServerFrame,
  type ProviderSelection,
} from '@demicodes/agent/client'
import { deferred, delay } from '@demicodes/utils'
import { ConversationRuntime, type RuntimeState } from '../conversation-runtime'

const provider: ProviderSelection = {
  providerId: 'stub',
  model: {
    providerId: 'stub',
    model: {
      id: 'stub',
      name: 'Stub',
      contextWindow: 1000,
      outputLimit: null,
      inputLimit: null,
      thinking: [],
      acceptedExtensions: [],
    },
    thinking: null,
  },
}
function state(): RuntimeState {
  return {
    id: 'conversation',
    cwd: '/',
    blocks: [],
    phase: 'idle',
    queue: [],
    pendingSteers: [],
    model: {
      providerId: 'stub',
      modelId: 'stub',
      thinkingEffort: null,
      serviceTierId: null,
    },
    lastError: null,
    load: 'loading',
  }
}
function clientHarness() {
  const sent: ClientFrame[] = []
  let receive: (frame: ServerFrame) => void = () => {}
  const client = new AgentClient({
    send(frame) {
      sent.push(frame)
      if (frame.type === 'open') {
        receive({ type: 'opened' })
      }
    },
    close() {},
    onFrame(handler) {
      receive = handler
      return () => {
        receive = () => {}
      }
    },
  })
  return {
    client,
    sent,
    receive: (frame: ServerFrame) => receive(frame),
  }
}

test('disposing a view during model preparation prevents a late connection', async () => {
  const prepared = deferred<ProviderSelection>()
  let connects = 0
  const runtime = new ConversationRuntime({
    state: state(),
    prepareModel: () => prepared.promise,
    connect: async () => {
      connects += 1
      return clientHarness().client
    },
  })
  const opening = runtime.connect()
  const result = opening.catch((error) => error)
  runtime.dispose()
  prepared.resolve(provider)
  expect(await result).toBeInstanceOf(Error)
  expect(connects).toBe(0)
})

test('disposing an open view detaches without sending task-close or abort commands', async () => {
  const h = clientHarness()
  const current = state()
  const runtime = new ConversationRuntime({
    state: current,
    prepareModel: async () => provider,
    connect: async () => h.client,
  })
  await runtime.connect()
  expect(current.load).toBe('ready')
  runtime.dispose()
  expect(h.sent.map((frame) => frame.type)).toEqual(['open'])
  await expect(runtime.connect()).rejects.toThrow('disposed')
})

test('server takeover does not start a reconnect fight between views', async () => {
  const h = clientHarness()
  let connects = 0
  const runtime = new ConversationRuntime({
    state: state(),
    prepareModel: async () => provider,
    connect: async () => {
      connects += 1
      return h.client
    },
  })
  await runtime.connect()
  h.receive({ type: 'closed' })
  await delay(1100)
  expect(connects).toBe(1)
  expect(runtime.connected).toBe(false)
  runtime.dispose()
})

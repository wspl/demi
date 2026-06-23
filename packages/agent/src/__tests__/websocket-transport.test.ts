import { expect, test } from 'bun:test'
import type { ModelSelection } from '@demi/core'
import type { AgentHarness } from '@demi/agent'
import { LocalHost } from '@demi/host-local'
import { ProviderRegistry, type AgentProvider, type InferenceRequest, type ProviderEvent } from '@demi/provider'
import { StubProvider, events } from '@demi/provider/testing'
import {
  AgentClient,
  AgentServer,
  createWebSocketClientTransport,
  createWebSocketServerTransport,
  type ClientSessionEvent,
  type ClientFrame,
  type JsonWebSocket,
  type ProviderConfig,
  type ServerFrame,
} from '../index'

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

test('WebSocket transports serialize frames as JSON text messages and preserve binary fields', async () => {
  const [clientSocket, serverSocket] = createSocketPair()
  const client = createWebSocketClientTransport(clientSocket)
  const server = createWebSocketServerTransport(serverSocket)

  const serverFrame = nextFrame<ClientFrame>(server)
  client.send({ type: 'send', messageId: 'ws-send-1', content: [{ type: 'text', text: 'hello' }] })
  expect(await serverFrame).toEqual({ type: 'send', messageId: 'ws-send-1', content: [{ type: 'text', text: 'hello' }] })

  const clientFrame = nextFrame<ServerFrame>(client)
  server.send({ type: 'phase', phase: 'running' })
  expect(await clientFrame).toEqual({ type: 'phase', phase: 'running' })

  const serverBinaryFrame = nextFrame<ClientFrame>(server)
  client.send({
    type: 'send',
    messageId: 'ws-send-2',
    content: [
      {
        type: 'image',
        source: {
          type: 'binary',
          data: new Uint8Array([4, 5, 6]),
          mediaType: 'image/png',
        },
      },
    ],
  })
  const binarySend = await serverBinaryFrame
  if (binarySend.type !== 'send') throw new Error('expected send frame')
  const sentContent = binarySend.content[0]
  if (sentContent?.type !== 'image' || sentContent.source.type !== 'binary') throw new Error('expected binary image')
  expect(sentContent.source.data).toBeInstanceOf(Uint8Array)
  expect([...sentContent.source.data]).toEqual([4, 5, 6])

  const clientSnapshotFrame = nextFrame<ServerFrame>(client)
  server.send({
    type: 'transcript_snapshot',
    blocks: [
      {
        type: 'user',
        id: 'user-1',
        turnId: 'turn-1',
        createdAt: '2026-06-17T00:00:00.000Z',
        model,
        content: [
          {
            type: 'document',
            source: {
              data: new Uint8Array([7, 8, 9]),
              mediaType: 'application/pdf',
              fileName: 'sample.pdf',
            },
          },
        ],
        preamble: null,
      },
    ],
  })
  const snapshot = await clientSnapshotFrame
  if (snapshot.type !== 'transcript_snapshot') throw new Error('expected transcript snapshot')
  const block = snapshot.blocks[0]
  if (block?.type !== 'user') throw new Error('expected user block')
  const receivedContent = block.content[0]
  if (receivedContent?.type !== 'document') throw new Error('expected document')
  expect(receivedContent.source.data).toBeInstanceOf(Uint8Array)
  expect([...receivedContent.source.data]).toEqual([7, 8, 9])
})

test('WebSocket transports carry AgentClient and AgentServer traffic end to end', async () => {
  const [clientSocket, serverSocket] = createSocketPair()
  const providerRegistry = new ProviderRegistry()
  providerRegistry.register({
    type: 'ws-stub',
    displayName: 'WebSocket Stub',
    createProvider: (config: unknown) => {
      const text = (config as { text: string }).text
      return new StubProvider([[events.text(text), events.response()]])
    },
  })

  const server = new AgentServer({
    agent: createHarness(),
    providerRegistry,
  })
  server.attachTransport(createWebSocketServerTransport(serverSocket))
  const client = new AgentClient(createWebSocketClientTransport(clientSocket))

  await client.open(providerConfig('over websocket'), '/workspace')
  await client.send([{ type: 'text', text: 'hello' }])
  await waitFor(() => client.transcript().blocks.some((block) => block.type === 'response'))

  expect(client.transcript().blocks.map((block) => block.type)).toEqual(['user', 'text', 'response'])
  const textBlock = client.transcript().blocks.find((block) => block.type === 'text')
  expect(textBlock).toMatchObject({ type: 'text', text: 'over websocket' })

  await client.close()
})

test('WebSocket transports preserve complex AgentClient action convergence', async () => {
  const [clientSocket, serverSocket] = createSocketPair()
  const provider = new WebSocketScenarioProvider()
  const providerRegistry = new ProviderRegistry()
  providerRegistry.register({
    type: 'ws-scenario',
    displayName: 'WebSocket Scenario',
    createProvider: () => provider,
  })

  const server = new AgentServer({
    agent: createHarness(),
    providerRegistry,
  })
  server.attachTransport(createWebSocketServerTransport(serverSocket))
  const client = new AgentClient(createWebSocketClientTransport(clientSocket))
  const seen: ClientSessionEvent[] = []
  client.subscribe((event) => seen.push(event))

  await client.open({ type: 'ws-scenario', model }, '/workspace')
  const first = client.send([{ type: 'text', text: 'first' }])
  await provider.firstStarted.promise
  const second = client.send([{ type: 'text', text: 'second ' + 'x'.repeat(20_000) }])
  await waitFor(() => seen.some((event) => event.type === 'queue' && event.queue.some((message) => message.text.startsWith('second'))))

  provider.firstRelease.resolve(undefined)
  await first
  await expect(second).rejects.toThrow('second failed')

  await client.retry()

  const aborting = client.send([{ type: 'text', text: 'abort me' }])
  await provider.abortStarted.promise
  await expect(client.abort()).resolves.toBe(true)
  await aborting
  await client.resume()
  await client.compact()
  await client.send([{ type: 'text', text: 'after compact' }])

  expect(provider.summaryRequests).toBe(1)
  expect(provider.afterCompactRequest?.items[0]).toEqual({
    type: 'user_message',
    content: [{ type: 'text', text: 'Previous conversation summary:\nwebsocket summary' }],
  })
  expect(client.transcript().blocks.at(-2)).toMatchObject({ type: 'text', text: 'after compact answer' })

  await client.close()
})

function nextFrame<T>(transport: { onFrame(handler: (frame: T) => void): () => void }): Promise<T> {
  return new Promise((resolve) => {
    const unsubscribe = transport.onFrame((frame) => {
      unsubscribe()
      resolve(frame)
    })
  })
}

function createHarness(): AgentHarness<Record<string, never>> {
  return {
    name: 'test',
    initialState: () => ({}),
    host: (ctx) => new LocalHost(ctx.cwd),
    systemPrompt: () => 'system',
  }
}

function providerConfig(text: string): ProviderConfig {
  return {
    type: 'ws-stub',
    config: { text },
    model,
  }
}

async function waitFor(predicate: () => boolean): Promise<void> {
  const startedAt = Date.now()
  while (!predicate()) {
    if (Date.now() - startedAt > 1_000) throw new Error('Timed out waiting for predicate')
    await new Promise((resolve) => setTimeout(resolve, 1))
  }
}

class WebSocketScenarioProvider implements AgentProvider {
  readonly firstStarted = deferred<void>()
  readonly firstRelease = deferred<void>()
  readonly abortStarted = deferred<void>()
  summaryRequests = 0
  afterCompactRequest: InferenceRequest | null = null
  private normalCalls = 0

  async *run(request: InferenceRequest): AsyncIterable<ProviderEvent> {
    if (request.systemPrompt.includes('Summarize the previous conversation')) {
      this.summaryRequests += 1
      yield events.text('websocket summary')
      yield events.response()
      return
    }

    const call = this.normalCalls
    this.normalCalls += 1

    if (call === 0) {
      this.firstStarted.resolve(undefined)
      await this.firstRelease.promise
      yield events.text('first answer')
      yield events.response()
      return
    }
    if (call === 1) {
      if (!latestUserText(request).startsWith('second')) throw new Error('queued send did not reach provider')
      yield events.error('second failed', 'test')
      return
    }
    if (call === 2) {
      if (!latestUserText(request).startsWith('second')) throw new Error('retry did not replay latest user')
      yield events.text(`retry answer ${'y'.repeat(20_000)}`)
      yield events.response()
      return
    }
    if (call === 3) {
      this.abortStarted.resolve(undefined)
      await new Promise<void>((resolve) => request.cancel.addEventListener('abort', () => resolve(), { once: true }))
      return
    }
    if (call === 4) {
      const latest = request.items.at(-1)
      if (
        latest?.type !== 'user_message' ||
        latest.content[0]?.type !== 'text' ||
        latest.content[0].text !== 'Continue from where you left off.'
      ) {
        throw new Error('resume request did not include resume marker')
      }
      yield events.text('resume answer')
      yield events.response()
      return
    }

    this.afterCompactRequest = request
    yield events.text('after compact answer')
    yield events.response()
  }
}

function latestUserText(request: InferenceRequest): string {
  const latest = [...request.items].reverse().find((item) => item.type === 'user_message')
  if (latest?.type !== 'user_message') return ''
  const textBlock = [...latest.content].reverse().find((block) => block.type === 'text')
  return textBlock?.type === 'text' ? textBlock.text : ''
}

function deferred<T>(): { promise: Promise<T>; resolve: (value: T) => void } {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((innerResolve) => {
    resolve = innerResolve
  })
  return { promise, resolve }
}

function createSocketPair(): [FakeSocket, FakeSocket] {
  const a = new FakeSocket()
  const b = new FakeSocket()
  a.peer = b
  b.peer = a
  return [a, b]
}

class FakeSocket implements JsonWebSocket {
  peer: FakeSocket | null = null
  private readonly messageListeners = new Set<(event: { data: unknown }) => void>()

  send(data: string): void {
    this.peer?.receive(data)
  }

  close(): void {
    this.messageListeners.clear()
  }

  addEventListener(type: 'message', listener: (event: { data: unknown }) => void): void {
    if (type === 'message') this.messageListeners.add(listener)
  }

  removeEventListener(type: 'message', listener: (event: { data: unknown }) => void): void {
    if (type === 'message') this.messageListeners.delete(listener)
  }

  private receive(data: string): void {
    queueMicrotask(() => {
      for (const listener of this.messageListeners) listener({ data })
    })
  }
}

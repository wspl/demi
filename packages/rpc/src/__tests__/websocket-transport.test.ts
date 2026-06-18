import { expect, test } from 'bun:test'
import type { AgentDefinition } from '@demi/base-agent'
import type { ModelSelection } from '@demi/core'
import { ProviderRegistry, StubProvider, events } from '@demi/provider'
import {
  RpcClient,
  RpcHost,
  createWebSocketClientTransport,
  createWebSocketHostTransport,
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
  const [clientSocket, hostSocket] = createSocketPair()
  const client = createWebSocketClientTransport(clientSocket)
  const host = createWebSocketHostTransport(hostSocket)

  const hostFrame = nextFrame<ClientFrame>(host)
  client.send({ type: 'send', content: [{ type: 'text', text: 'hello' }] })
  expect(await hostFrame).toEqual({ type: 'send', content: [{ type: 'text', text: 'hello' }] })

  const clientFrame = nextFrame<ServerFrame>(client)
  host.send({ type: 'phase', phase: 'running' })
  expect(await clientFrame).toEqual({ type: 'phase', phase: 'running' })

  const hostBinaryFrame = nextFrame<ClientFrame>(host)
  client.send({
    type: 'send',
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
  const binarySend = await hostBinaryFrame
  if (binarySend.type !== 'send') throw new Error('expected send frame')
  const sentContent = binarySend.content[0]
  if (sentContent?.type !== 'image' || sentContent.source.type !== 'binary') throw new Error('expected binary image')
  expect(sentContent.source.data).toBeInstanceOf(Uint8Array)
  expect([...sentContent.source.data]).toEqual([4, 5, 6])

  const clientSnapshotFrame = nextFrame<ServerFrame>(client)
  host.send({
    type: 'transcript_snapshot',
    blocks: [
      {
        type: 'user',
        id: 'user-1',
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

test('WebSocket transports carry RpcClient and RpcHost traffic end to end', async () => {
  const [clientSocket, hostSocket] = createSocketPair()
  const providerRegistry = new ProviderRegistry()
  providerRegistry.register({
    type: 'ws-stub',
    displayName: 'WebSocket Stub',
    createProvider: (config: unknown) => {
      const text = (config as { text: string }).text
      return new StubProvider([[events.text(text), events.response()]])
    },
  })

  new RpcHost({
    transport: createWebSocketHostTransport(hostSocket),
    providerRegistry,
    definitions: { test: createDefinition() },
  })
  const client = new RpcClient(createWebSocketClientTransport(clientSocket))

  await client.open('test', providerConfig('over websocket'), '/workspace')
  await client.send([{ type: 'text', text: 'hello' }])
  await waitFor(() => client.transcript().blocks.some((block) => block.type === 'response'))

  expect(client.transcript().blocks.map((block) => block.type)).toEqual(['user', 'text', 'response'])
  const textBlock = client.transcript().blocks.find((block) => block.type === 'text')
  expect(textBlock).toMatchObject({ type: 'text', text: 'over websocket' })

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

function createDefinition(): AgentDefinition<Record<string, never>> {
  return {
    name: 'test',
    initialState: () => ({}),
    systemPrompt: () => 'system',
    tools: () => [],
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

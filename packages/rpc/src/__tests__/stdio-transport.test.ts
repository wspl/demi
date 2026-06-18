import { PassThrough } from 'node:stream'
import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { dirname, join } from 'node:path'
import { expect, test } from 'bun:test'
import type { ModelSelection } from '@demi/core'
import type { AgentDefinition } from '@demi/base-agent'
import { ProviderRegistry, StubProvider, events } from '@demi/provider'
import {
  RpcClient,
  RpcHost,
  type ClientFrame,
  type ProviderConfig,
} from '../index'
import { createStdioClientTransport, createStdioHostTransport } from '../stdio-transport'

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

test('StdioTransport preserves Uint8Array fields through JSON frames', async () => {
  const clientToHost = new PassThrough()
  const hostToClient = new PassThrough()
  const host = createStdioHostTransport(clientToHost, hostToClient)
  const client = createStdioClientTransport(hostToClient, clientToHost)
  const received = nextFrame<ClientFrame>(host)

  client.send({
    type: 'send',
    content: [
      {
        type: 'image',
        source: {
          type: 'binary',
          data: new Uint8Array([1, 2, 3]),
          mediaType: 'image/png',
        },
      },
    ],
  })

  const frame = await received
  if (frame.type !== 'send') throw new Error('expected send frame')
  const content = frame.content[0]
  if (content?.type !== 'image' || content.source.type !== 'binary') throw new Error('expected binary image')
  expect(content.source.data).toBeInstanceOf(Uint8Array)
  expect([...content.source.data]).toEqual([1, 2, 3])

  client.close()
  host.close()
})

test('StdioTransport carries the same RpcClient/RpcHost frames over NDJSON', async () => {
  const clientToHost = new PassThrough()
  const hostToClient = new PassThrough()
  const providerRegistry = new ProviderRegistry()
  providerRegistry.register({
    type: 'echo-stub',
    displayName: 'Echo Stub',
    createProvider: (config: unknown) => {
      const text = (config as { text: string }).text
      return new StubProvider([[events.text(text), events.response()]])
    },
  })

  new RpcHost({
    transport: createStdioHostTransport(clientToHost, hostToClient),
    providerRegistry,
    definitions: { test: createDefinition() },
  })
  const client = new RpcClient(createStdioClientTransport(hostToClient, clientToHost))

  await client.open('test', providerConfig('over stdio'), '/workspace')
  await client.send([{ type: 'text', text: 'hello' }])
  await waitFor(() => client.transcript().blocks.some((block) => block.type === 'response'))

  expect(client.transcript().blocks.map((block) => block.type)).toEqual(['user', 'text', 'response'])
  const textBlock = client.transcript().blocks.find((block) => block.type === 'text')
  expect(textBlock).toMatchObject({ type: 'text', text: 'over stdio' })
})

test('StdioTransport carries RpcClient frames to a child-process RpcHost', async () => {
  const child = spawn(process.execPath, [join(dirname(fileURLToPath(import.meta.url)), 'stdio-host-fixture.ts')], {
    cwd: process.cwd(),
    env: process.env,
    stdio: ['pipe', 'pipe', 'pipe'],
  })
  let stderr = ''
  child.stderr.on('data', (chunk) => {
    stderr += Buffer.from(chunk).toString('utf8')
  })

  const client = new RpcClient(createStdioClientTransport(child.stdout, child.stdin))
  try {
    await withTimeout(client.open('test', childProviderConfig('from child'), '/workspace'), 'open child session', () => stderr)
    await client.send([{ type: 'text', text: 'hello child' }])
    await waitFor(() => client.transcript().blocks.some((block) => block.type === 'response'), () => stderr)

    expect(client.transcript().blocks.map((block) => block.type)).toEqual(['user', 'text', 'response'])
    const textBlock = client.transcript().blocks.find((block) => block.type === 'text')
    expect(textBlock).toMatchObject({ type: 'text', text: 'from child' })

    await withTimeout(client.close(), 'close child session', () => stderr)
  } finally {
    await stopChild(child)
  }
})

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
    type: 'echo-stub',
    config: { text },
    model,
  }
}

function childProviderConfig(text: string): ProviderConfig {
  return {
    type: 'child-stub',
    config: { text },
    model,
  }
}

function nextFrame<T>(transport: { onFrame(handler: (frame: T) => void): () => void }): Promise<T> {
  return new Promise((resolve) => {
    const unsubscribe = transport.onFrame((frame) => {
      unsubscribe()
      resolve(frame)
    })
  })
}

async function waitForWithError(predicate: () => boolean, errorDetails: () => string): Promise<void> {
  const startedAt = Date.now()
  while (!predicate()) {
    if (Date.now() - startedAt > 1_000) {
      const details = errorDetails().trim()
      throw new Error(`Timed out waiting for predicate${details ? `: ${details}` : ''}`)
    }
    await new Promise((resolve) => setTimeout(resolve, 1))
  }
}

function waitFor(predicate: () => boolean, errorDetails?: () => string): Promise<void> {
  return errorDetails ? waitForWithError(predicate, errorDetails) : waitForWithError(predicate, () => '')
}

function withTimeout<T>(promise: Promise<T>, label: string, errorDetails: () => string): Promise<T> {
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => {
      const details = errorDetails().trim()
      reject(new Error(`Timed out during ${label}${details ? `: ${details}` : ''}`))
    }, 1_000)
    promise.then(
      (value) => {
        clearTimeout(timeout)
        resolve(value)
      },
      (error) => {
        clearTimeout(timeout)
        reject(error)
      },
    )
  })
}

async function stopChild(child: ChildProcessWithoutNullStreams): Promise<void> {
  if (child.exitCode !== null || child.signalCode !== null) return
  child.kill()
  await new Promise<void>((resolve) => {
    const timeout = setTimeout(resolve, 100)
    child.once('exit', () => {
      clearTimeout(timeout)
      resolve()
    })
  })
}

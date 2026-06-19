import { PassThrough } from 'node:stream'
import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process'
import { access, mkdtemp } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { fileURLToPath } from 'node:url'
import { dirname, join } from 'node:path'
import { expect, test } from 'bun:test'
import type { ModelSelection } from '@demi/core'
import type { AgentHarness } from '@demi/agent'
import { LocalHost } from '@demi/shell/local-host'
import { ProviderRegistry, StubProvider, events, type AgentProvider, type InferenceRequest, type ProviderEvent } from '@demi/provider'
import {
  AgentClient,
  AgentServer,
  type ClientSessionEvent,
  type ClientFrame,
  type ProviderConfig,
} from '../index'
import { createStdioClientTransport, createStdioServerTransport } from '../stdio-transport'

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
  const clientToServer = new PassThrough()
  const serverToClient = new PassThrough()
  const server = createStdioServerTransport(clientToServer, serverToClient)
  const client = createStdioClientTransport(serverToClient, clientToServer)
  const received = nextFrame<ClientFrame>(server)

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
  server.close()
})

test('StdioTransport carries the same AgentClient/AgentServer frames over NDJSON', async () => {
  const clientToServer = new PassThrough()
  const serverToClient = new PassThrough()
  const providerRegistry = new ProviderRegistry()
  providerRegistry.register({
    type: 'echo-stub',
    displayName: 'Echo Stub',
    createProvider: (config: unknown) => {
      const text = (config as { text: string }).text
      return new StubProvider([[events.text(text), events.response()]])
    },
  })

  const server = new AgentServer({
    agent: createHarness(),
    providerRegistry,
  })
  server.attachTransport(createStdioServerTransport(clientToServer, serverToClient))
  const client = new AgentClient(createStdioClientTransport(serverToClient, clientToServer))

  await client.open(providerConfig('over stdio'), '/workspace')
  await client.send([{ type: 'text', text: 'hello' }])
  await waitFor(() => client.transcript().blocks.some((block) => block.type === 'response'))

  expect(client.transcript().blocks.map((block) => block.type)).toEqual(['user', 'text', 'response'])
  const textBlock = client.transcript().blocks.find((block) => block.type === 'text')
  expect(textBlock).toMatchObject({ type: 'text', text: 'over stdio' })
})

test('StdioTransport preserves complex AgentClient action convergence over NDJSON', async () => {
  const clientToServer = new PassThrough()
  const serverToClient = new PassThrough()
  const provider = new StdioScenarioProvider()
  const providerRegistry = new ProviderRegistry()
  providerRegistry.register({
    type: 'stdio-scenario',
    displayName: 'Stdio Scenario',
    createProvider: () => provider,
  })

  const server = new AgentServer({
    agent: createHarness(),
    providerRegistry,
  })
  server.attachTransport(createStdioServerTransport(clientToServer, serverToClient))
  const client = new AgentClient(createStdioClientTransport(serverToClient, clientToServer))
  const seen: ClientSessionEvent[] = []
  client.subscribe((event) => seen.push(event))

  await client.open({ type: 'stdio-scenario', model }, '/workspace')
  const first = client.send([{ type: 'text', text: 'first ' + 'x'.repeat(20_000) }])
  await provider.firstStarted.promise
  const second = client.send([{ type: 'text', text: 'second' }])
  await waitFor(() => seen.some((event) => event.type === 'queue' && event.queue.some((message) => message.text === 'second')))

  provider.firstRelease.resolve(undefined)
  await first
  await expect(second).rejects.toThrow('second failed')
  expect(client.transcript().blocks.some((block) => block.type === 'error')).toBe(true)

  await client.retry()
  expect(client.transcript().blocks.some((block) => block.type === 'error')).toBe(false)

  const aborting = client.send([{ type: 'text', text: 'abort me' }])
  await provider.abortStarted.promise
  await expect(client.abort()).resolves.toBe(true)
  await aborting
  await client.resume()

  await client.compact()
  expect(client.transcript().blocks.some((block) => block.type === 'compaction_boundary')).toBe(true)

  await client.send([{ type: 'text', text: 'after compact' }])

  expect(provider.summaryRequests).toBe(1)
  expect(provider.afterCompactRequest?.items[0]).toEqual({
    type: 'user_message',
    content: [{ type: 'text', text: 'Previous conversation summary:\nstdio summary' }],
  })
  expect(client.transcript().blocks.at(-2)).toMatchObject({ type: 'text', text: 'after compact answer' })

  await server.close()
})

test('StdioTransport close disposes shell foreground processes through AgentServer', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-stdio-close-shell-'))
  const leakedPath = join(root, 'stdio-leaked.txt')
  const clientToServer = new PassThrough()
  const serverToClient = new PassThrough()
  const providerRegistry = new ProviderRegistry()
  providerRegistry.register({
    type: 'stdio-shell',
    displayName: 'Stdio Shell',
    createProvider: () =>
      new StubProvider([
        [
          events.toolCall('tool-1', 'shell_exec', {
            script: 'sh -c "sleep 0.2; printf leaked > stdio-leaked.txt"',
            yieldAfterMs: 1,
          }),
        ],
        [events.text('running'), events.response()],
      ]),
  })
  const server = new AgentServer({
    agent: createHarness(),
    providerRegistry,
    shell: {
      initialEnv: { PATH: process.env.PATH ?? '' },
      shellIdFactory: () => 'stdio-close-shell',
    },
  })
  server.attachTransport(createStdioServerTransport(clientToServer, serverToClient))
  const client = new AgentClient(createStdioClientTransport(serverToClient, clientToServer))

  await client.open({ type: 'stdio-shell', model }, root)
  await client.send([{ type: 'text', text: 'start long command' }])
  await waitFor(() => client.transcript().blocks.some((block) => block.type === 'response'))

  await client.close()

  await delay(250)
  await expect(access(leakedPath)).rejects.toThrow()
})

test('StdioTransport carries AgentClient frames to a child-process AgentServer', async () => {
  const child = spawn(process.execPath, [join(dirname(fileURLToPath(import.meta.url)), 'stdio-host-fixture.ts')], {
    cwd: process.cwd(),
    env: process.env,
    stdio: ['pipe', 'pipe', 'pipe'],
  })
  let stderr = ''
  child.stderr.on('data', (chunk) => {
    stderr += Buffer.from(chunk).toString('utf8')
  })

  const client = new AgentClient(createStdioClientTransport(child.stdout, child.stdin))
  try {
    await withTimeout(client.open(childProviderConfig('from child'), '/workspace'), 'open child session', () => stderr)
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

class StdioScenarioProvider implements AgentProvider {
  readonly firstStarted = deferred<void>()
  readonly firstRelease = deferred<void>()
  readonly abortStarted = deferred<void>()
  summaryRequests = 0
  afterCompactRequest: InferenceRequest | null = null
  private normalCalls = 0

  async *run(request: InferenceRequest): AsyncIterable<ProviderEvent> {
    if (request.systemPrompt.includes('Summarize the previous conversation')) {
      this.summaryRequests += 1
      yield events.text('stdio summary')
      yield events.response()
      return
    }

    const call = this.normalCalls
    this.normalCalls += 1

    if (call === 0) {
      this.firstStarted.resolve(undefined)
      await this.firstRelease.promise
      yield events.text(`first answer ${'y'.repeat(20_000)}`)
      yield events.response()
      return
    }
    if (call === 1) {
      if (latestUserText(request) !== 'second') throw new Error('second request did not preserve queued user')
      yield events.error('second failed', 'test')
      return
    }
    if (call === 2) {
      if (latestUserText(request) !== 'second') throw new Error('retry did not replay latest user')
      yield events.text('retry answer')
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

function delay(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms))
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
